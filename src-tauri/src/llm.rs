//! Thin client for a locally-running Ollama server.
//!
//! Everything here talks to `http://127.0.0.1:11434`. If nothing is listening
//! there we start `ollama serve` ourselves — preferring the copy vendored under
//! `src-tauri/vendor/ollama-dist/`, falling back to whatever is on `PATH`.

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

const BASE: &str = "http://127.0.0.1:11434";

// ---------- wire types ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Present on `role = "tool"` messages so Ollama can correlate the result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self::plain("system", content)
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self::plain("user", content)
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::plain("assistant", content)
    }
    pub fn tool(name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
            tool_calls: None,
            tool_name: Some(name.into()),
        }
    }
    fn plain(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_calls: None,
            tool_name: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    /// Ollama returns this as a JSON object, not a string.
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Deserialize)]
struct ChatApiResponse {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct GenerateApiResponse {
    response: String,
}

#[derive(Debug, Deserialize)]
struct TagsResponse {
    models: Vec<TagModel>,
}

#[derive(Debug, Deserialize)]
struct TagModel {
    name: String,
}

// ---------- lifecycle ----------

fn vendored_ollama() -> Option<PathBuf> {
    // Dev builds run from `src-tauri/`; `CARGO_MANIFEST_DIR` is baked in at
    // compile time and points there on this machine.
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("vendor/ollama-dist/ollama");
    p.exists().then_some(p)
}

async fn server_up(http: &reqwest::Client) -> bool {
    http.get(format!("{BASE}/api/version"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Makes sure a server is reachable, starting one if needed. Safe to call often;
/// it no-ops once something is listening.
pub async fn ensure_running(state: &AppState) -> Result<(), String> {
    if server_up(&state.http).await {
        return Ok(());
    }

    let mut cmd = match vendored_ollama() {
        Some(bin) => {
            let mut c = tokio::process::Command::new(&bin);
            if let Some(dir) = bin.parent() {
                // The vendored binary needs its sibling dylibs on the loader path.
                c.env("DYLD_LIBRARY_PATH", dir);
            }
            c
        }
        None => tokio::process::Command::new("ollama"),
    };
    cmd.arg("serve")
        .kill_on_drop(true)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    let child = cmd
        .spawn()
        .map_err(|e| format!("could not start a local AI server (ollama): {e}"))?;
    *state.ollama_child.lock().map_err(|e| e.to_string())? = Some(child);

    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if server_up(&state.http).await {
            return Ok(());
        }
    }
    Err("local AI server did not come up within 20s".into())
}

/// Kills the server we spawned (if any). Called on app exit.
pub fn shutdown(state: &AppState) {
    if let Ok(mut guard) = state.ollama_child.lock() {
        if let Some(mut child) = guard.take() {
            let _ = child.start_kill();
        }
    }
}

// ---------- inference ----------

fn strip_think(s: &str) -> String {
    // Reasoning models (qwen3) wrap chain-of-thought in <think>…</think>. We ask
    // for `think:false` below, but strip defensively in case the model ignores it.
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</think>") {
            Some(end) => rest = &rest[start + end + "</think>".len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// One assistant turn. `tools` is an Ollama-format tool array (may be empty).
pub async fn chat(
    http: &reqwest::Client,
    model: &str,
    messages: &[ChatMessage],
    tools: &[Value],
) -> Result<ChatMessage, String> {
    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "tools": tools,
        "stream": false,
        "think": false,
        "options": { "temperature": 0.2 }
    });

    let resp = http
        .post(format!("{BASE}/api/chat"))
        .json(&body)
        .timeout(Duration::from_secs(180))
        .send()
        .await
        .map_err(|e| format!("local AI request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("local AI error {status}: {text}"));
    }

    let mut parsed: ChatApiResponse = resp
        .json()
        .await
        .map_err(|e| format!("could not parse local AI response: {e}"))?;
    parsed.message.content = strip_think(&parsed.message.content);
    Ok(parsed.message)
}

/// One-shot completion with no tools or history — used for summaries and triage.
pub async fn generate(http: &reqwest::Client, model: &str, prompt: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "stream": false,
        "think": false,
        "options": { "temperature": 0.1 }
    });

    let resp = http
        .post(format!("{BASE}/api/generate"))
        .json(&body)
        .timeout(Duration::from_secs(180))
        .send()
        .await
        .map_err(|e| format!("local AI request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("local AI error {status}: {text}"));
    }

    let parsed: GenerateApiResponse = resp
        .json()
        .await
        .map_err(|e| format!("could not parse local AI response: {e}"))?;
    Ok(strip_think(&parsed.response))
}

pub async fn list_models(http: &reqwest::Client) -> Result<Vec<String>, String> {
    let resp = http
        .get(format!("{BASE}/api/tags"))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| format!("could not reach local AI server: {e}"))?;
    let parsed: TagsResponse = resp
        .json()
        .await
        .map_err(|e| format!("could not parse model list: {e}"))?;
    Ok(parsed.models.into_iter().map(|m| m.name).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_think_removes_a_single_block() {
        assert_eq!(
            strip_think("<think>weighing options</think>The answer is 4."),
            "The answer is 4."
        );
    }

    #[test]
    fn strip_think_removes_multiple_blocks_and_keeps_the_gaps() {
        assert_eq!(
            strip_think("<think>a</think>Hello <think>b</think>world"),
            "Hello world"
        );
    }

    #[test]
    fn strip_think_drops_an_unclosed_block_to_end_of_string() {
        assert_eq!(strip_think("visible<think>never closed"), "visible");
    }

    #[test]
    fn strip_think_is_a_noop_without_tags() {
        assert_eq!(strip_think("  plain text  "), "plain text");
    }

    #[test]
    fn strip_think_handles_content_before_the_tag() {
        assert_eq!(strip_think("Answer: <think>hm</think>42"), "Answer: 42");
    }

    #[test]
    fn chat_message_constructors_set_role_and_leave_tool_fields_empty() {
        let m = ChatMessage::user("hi");
        assert_eq!(m.role, "user");
        assert!(m.tool_calls.is_none());
        assert!(m.tool_name.is_none());

        let t = ChatMessage::tool("search_emails", "{}");
        assert_eq!(t.role, "tool");
        assert_eq!(t.tool_name.as_deref(), Some("search_emails"));
    }

    #[test]
    fn tool_messages_serialize_with_tool_name_but_no_null_tool_calls() {
        let json = serde_json::to_string(&ChatMessage::tool("get_email", "{\"id\":1}")).unwrap();
        assert!(json.contains("\"tool_name\":\"get_email\""));
        assert!(!json.contains("tool_calls"));
    }
}
