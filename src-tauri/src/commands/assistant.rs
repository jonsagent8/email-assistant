//! The natural-language assistant. Runs a bounded tool-calling loop against the
//! local model, where every tool is a read/summarize/draft operation over the
//! locally-cached mailbox. Nothing here sends mail.

use crate::state::AppState;
use crate::{ai, llm};
use crate::llm::ChatMessage;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::State;

const MAX_ROUNDS: usize = 6;
const HISTORY_LIMIT: i64 = 20;

#[derive(Serialize)]
pub struct AssistantReply {
    pub text: String,
    /// Human-readable notes on what the assistant did ("searched inbox for …").
    pub actions: Vec<String>,
}

fn system_prompt() -> ChatMessage {
    ChatMessage::system(
        "You are a private email assistant running locally on the user's computer. \
         You can only see the user's cached INBOX through the provided tools. \
         Always call a tool to look something up before answering questions about mail — \
         never guess sender names, dates, or contents. When the user asks for the last thing \
         someone said, use list_from_person. \
         \
         email_id values are opaque: never invent one. Before calling get_email, \
         summarize_email, or draft_reply, you must first obtain the id from a \
         search_emails or list_from_person result in this conversation. If the user names a \
         person (\"reply to Priya\"), call list_from_person first to find the message, then \
         pass that id to draft_reply. If a lookup returns no results, say so instead of \
         guessing an id. \
         \
         Keep answers short and specific, and refer to messages by sender and subject. If you \
         draft a reply, tell the user it is waiting in the Drafts tab for their review — you \
         cannot send anything yourself.",
    )
}

fn tool_defs() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "search_emails",
                "description": "Search the cached inbox by keyword, sender, and/or earliest date.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "keyword to match in subject/body/sender"},
                        "from": {"type": "string", "description": "substring of the sender address or name"},
                        "since": {"type": "string", "description": "ISO date, only messages on/after it"},
                        "limit": {"type": "integer", "description": "max results, default 20"}
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "list_from_person",
                "description": "List the most recent messages from one person, newest first. Use for 'what did X last say'.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "person": {"type": "string", "description": "name or email substring"},
                        "limit": {"type": "integer", "description": "max results, default 5"}
                    },
                    "required": ["person"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_email",
                "description": "Get the full body of one cached message by its id.",
                "parameters": {
                    "type": "object",
                    "properties": {"email_id": {"type": "integer"}},
                    "required": ["email_id"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "summarize_email",
                "description": "Summarize one cached message by its id.",
                "parameters": {
                    "type": "object",
                    "properties": {"email_id": {"type": "integer"}},
                    "required": ["email_id"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "triage_recent",
                "description": "Classify recent un-triaged inbox messages (category, labels, action items).",
                "parameters": {
                    "type": "object",
                    "properties": {"limit": {"type": "integer", "description": "how many, default 20"}}
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "draft_reply",
                "description": "Write a reply draft for one message. It is saved for the user to review and send; it is NOT sent.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "email_id": {"type": "integer"},
                        "instructions": {"type": "string", "description": "what the reply should say"}
                    },
                    "required": ["email_id"]
                }
            }
        }),
    ]
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty())
}

fn arg_i64(args: &Value, key: &str) -> Option<i64> {
    match args.get(key) {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

fn first_account_id(state: &AppState) -> Result<i64, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.query_row("SELECT id FROM accounts ORDER BY id LIMIT 1", [], |r| r.get(0))
        .map_err(|_| "no mailbox is connected yet".to_string())
}

/// Compact rows for the model: id/from/subject/date/snippet only.
fn search_rows(
    state: &AppState,
    query: Option<&str>,
    from: Option<&str>,
    since: Option<&str>,
    limit: i64,
) -> Result<Value, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut sql = String::from(
        "SELECT id, from_addr, subject, date, snippet FROM emails WHERE folder = 'INBOX'",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(q) = query {
        sql.push_str(" AND (subject LIKE ? OR body_text LIKE ? OR from_addr LIKE ?)");
        let like = format!("%{q}%");
        params.push(Box::new(like.clone()));
        params.push(Box::new(like.clone()));
        params.push(Box::new(like));
    }
    if let Some(f) = from {
        sql.push_str(" AND from_addr LIKE ?");
        params.push(Box::new(format!("%{f}%")));
    }
    if let Some(s) = since {
        sql.push_str(" AND date >= ?");
        params.push(Box::new(s.to_string()));
    }
    sql.push_str(" ORDER BY date DESC, uid DESC LIMIT ?");
    params.push(Box::new(limit.clamp(1, 50)));

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(refs.as_slice(), |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "from": row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                "subject": row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                "date": row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                "snippet": row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            }))
        })
        .map_err(|e| e.to_string())?;
    let list: Vec<Value> = rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
    Ok(json!({ "results": list, "count": list.len() }))
}

/// Executes one tool call, returning `(json_result_string, human_label)`.
async fn run_tool(state: &AppState, name: &str, args: &Value) -> (String, String) {
    let outcome: Result<(Value, String), String> = match name {
        "search_emails" => {
            let q = arg_str(args, "query");
            let from = arg_str(args, "from");
            let since = arg_str(args, "since");
            let limit = arg_i64(args, "limit").unwrap_or(20);
            let label = match (q, from) {
                (Some(q), _) => format!("searched inbox for “{q}”"),
                (None, Some(f)) => format!("searched inbox from “{f}”"),
                _ => "listed recent inbox".to_string(),
            };
            search_rows(state, q, from, since, limit).map(|v| (v, label))
        }
        "list_from_person" => match arg_str(args, "person") {
            Some(p) => {
                let limit = arg_i64(args, "limit").unwrap_or(5);
                search_rows(state, None, Some(p), None, limit)
                    .map(|v| (v, format!("looked up messages from “{p}”")))
            }
            None => Err("person is required".into()),
        },
        "get_email" => match arg_i64(args, "email_id") {
            Some(id) => {
                let conn = match state.db.lock() {
                    Ok(c) => c,
                    Err(e) => return (json!({"error": e.to_string()}).to_string(), String::new()),
                };
                conn.query_row(
                    "SELECT from_addr, subject, date, body_text FROM emails WHERE id = ?1",
                    [id],
                    |row| {
                        let body: String = row.get::<_, Option<String>>(3)?.unwrap_or_default();
                        Ok(json!({
                            "id": id,
                            "from": row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                            "subject": row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                            "date": row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                            "body": body.chars().take(4000).collect::<String>(),
                        }))
                    },
                )
                .map(|v| (v, format!("opened email #{id}")))
                .map_err(|e| e.to_string())
            }
            None => Err("email_id is required".into()),
        },
        "summarize_email" => match arg_i64(args, "email_id") {
            Some(id) => ai::summarize_email(state, id)
                .await
                .map(|s| (json!({ "summary": s }), format!("summarized email #{id}"))),
            None => Err("email_id is required".into()),
        },
        "triage_recent" => {
            let limit = arg_i64(args, "limit").unwrap_or(20).clamp(1, 50);
            match first_account_id(state) {
                Ok(acc) => ai::triage(state, acc, limit).await.map(|r| {
                    (
                        json!({ "triaged": r.triaged }),
                        format!("triaged {} message(s)", r.triaged),
                    )
                }),
                Err(e) => Err(e),
            }
        }
        "draft_reply" => match arg_i64(args, "email_id") {
            Some(id) => {
                let instr = arg_str(args, "instructions").map(|s| s.to_string());
                crate::commands::drafts::generate_draft_internal(state, id, instr)
                    .await
                    .map(|d| {
                        (
                            json!({
                                "draft_id": d.id, "to": d.to, "subject": d.subject,
                                "body": d.draft_text, "status": "waiting in Drafts tab for review"
                            }),
                            format!("drafted a reply to “{}”", d.original_subject),
                        )
                    })
            }
            None => Err("email_id is required".into()),
        },
        other => Err(format!("unknown tool: {other}")),
    };

    match outcome {
        Ok((value, label)) => (value.to_string(), label),
        Err(e) => (json!({ "error": e }).to_string(), String::new()),
    }
}

fn ensure_session(state: &AppState, session_id: &str) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR IGNORE INTO chat_sessions (id) VALUES (?1)",
        [session_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn load_history(state: &AppState, session_id: &str) -> Result<Vec<ChatMessage>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT role, content FROM chat_messages
             WHERE session_id = ?1 AND role IN ('user', 'assistant')
             ORDER BY id DESC LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let mut rows: Vec<ChatMessage> = stmt
        .query_map(rusqlite::params![session_id, HISTORY_LIMIT], |row| {
            let role: String = row.get(0)?;
            let content: String = row.get(1)?;
            Ok(match role.as_str() {
                "assistant" => ChatMessage::assistant(content),
                _ => ChatMessage::user(content),
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    rows.reverse();
    Ok(rows)
}

fn save_message(
    state: &AppState,
    session_id: &str,
    role: &str,
    content: &str,
    trace: Option<&str>,
) {
    if let Ok(conn) = state.db.lock() {
        let _ = conn.execute(
            "INSERT INTO chat_messages (session_id, role, content, tool_trace)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, role, content, trace],
        );
    }
}

#[tauri::command]
pub async fn assistant_chat(
    state: State<'_, AppState>,
    session_id: String,
    message: String,
) -> Result<AssistantReply, String> {
    run_chat(&state, &session_id, &message).await
}

/// The assistant turn itself, taking a plain `&AppState` so it can be driven from
/// tests without a Tauri runtime.
async fn run_chat(
    state: &AppState,
    session_id: &str,
    message: &str,
) -> Result<AssistantReply, String> {
    llm::ensure_running(state).await?;
    ensure_session(state, session_id)?;

    let model = state.setting("chat_model", "qwen3:8b-q4_K_M");
    let tools = tool_defs();

    let mut messages = vec![system_prompt()];
    messages.extend(load_history(state, session_id)?);
    messages.push(ChatMessage::user(message));
    save_message(state, session_id, "user", message, None);

    let mut actions: Vec<String> = Vec::new();

    for _ in 0..MAX_ROUNDS {
        let reply = llm::chat(&state.http, &model, &messages, &tools).await?;
        let calls = reply.tool_calls.clone().unwrap_or_default();
        messages.push(reply.clone());

        if calls.is_empty() {
            let text = reply.content.trim().to_string();
            let trace = if actions.is_empty() {
                None
            } else {
                Some(actions.join(" · "))
            };
            save_message(state, session_id, "assistant", &text, trace.as_deref());
            return Ok(AssistantReply { text, actions });
        }

        for call in calls {
            let (result, label) = run_tool(state, &call.function.name, &call.function.arguments).await;
            if !label.is_empty() {
                actions.push(label);
            }
            messages.push(ChatMessage::tool(&call.function.name, result));
        }
    }

    let text = "I looked into that but couldn't wrap it up — try narrowing the question.".to_string();
    save_message(state, session_id, "assistant", &text, Some(&actions.join(" · ")));
    Ok(AssistantReply { text, actions })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arg_str_returns_non_empty_strings() {
        let args = json!({"query": "invoice", "from": "  ", "n": 3});
        assert_eq!(arg_str(&args, "query"), Some("invoice"));
        assert_eq!(arg_str(&args, "from"), None); // whitespace-only is treated as absent
        assert_eq!(arg_str(&args, "missing"), None);
        assert_eq!(arg_str(&args, "n"), None); // wrong type
    }

    #[test]
    fn arg_i64_accepts_numbers_and_numeric_strings() {
        let args = json!({"a": 5, "b": "12", "c": "nope", "d": 1.9});
        assert_eq!(arg_i64(&args, "a"), Some(5));
        assert_eq!(arg_i64(&args, "b"), Some(12)); // models often stringify ints
        assert_eq!(arg_i64(&args, "c"), None);
        assert_eq!(arg_i64(&args, "missing"), None);
        assert_eq!(arg_i64(&args, "d"), None); // non-integer float
    }

    #[test]
    fn tool_defs_expose_the_expected_six_tools() {
        let names: Vec<String> = tool_defs()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            names,
            [
                "search_emails",
                "list_from_person",
                "get_email",
                "summarize_email",
                "triage_recent",
                "draft_reply",
            ]
        );
    }

    #[test]
    fn every_tool_def_is_a_well_formed_function_schema() {
        for t in tool_defs() {
            assert_eq!(t["type"], "function");
            assert!(t["function"]["name"].is_string());
            assert_eq!(t["function"]["parameters"]["type"], "object");
            assert!(t["function"]["parameters"]["properties"].is_object());
        }
    }

    #[test]
    fn system_prompt_forbids_sending_and_guessing() {
        let p = system_prompt().content.to_lowercase();
        assert!(p.contains("cannot send") || p.contains("cannot send anything"));
        assert!(p.contains("never guess"));
    }

    // ---- live tests: require a local Ollama with the default chat model.
    // Run with:  cargo test -- --ignored --test-threads=1

    fn seeded_state() -> AppState {
        let conn = crate::db::memory_db();
        conn.execute(
            "INSERT INTO accounts (id, email, imap_host, imap_port, smtp_host, smtp_port)
             VALUES (1, 'me@example.com', 'imap.example.com', 993, 'smtp.example.com', 587)",
            [],
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO emails (id, account_id, folder, uid, from_addr, subject, date, snippet, body_text) VALUES
             (1, 1, 'INBOX', 101, 'Priya Shah <priya@vendor.test>', 'Q3 invoice attached',
              '2026-08-28T14:00:00+00:00', 'Invoice 4471 for 2,300 dollars, net 30.',
              'Hi, attached is invoice 4471 for the Q3 work, total $2,300, payable net 30. Let me know if the PO number is different. Best, Priya');
             INSERT INTO emails (id, account_id, folder, uid, from_addr, subject, date, snippet, body_text) VALUES
             (2, 1, 'INBOX', 102, 'Marco Vidal <marco@team.test>', 'Re: offsite dates',
              '2026-08-29T18:30:00+00:00', 'Locking in Oct 14-16 for the offsite.',
              'Talked to the venue - Oct 14 to 16 works and the rate holds if we confirm this week. Want me to book it? - Marco');",
        )
        .unwrap();
        AppState::new(conn)
    }

    #[tokio::test]
    #[ignore = "requires local Ollama"]
    async fn live_assistant_uses_a_tool_before_answering_about_mail() {
        let state = seeded_state();
        let reply = run_chat(&state, "s-tool", "What did Marco say about the offsite?")
            .await
            .expect("assistant replies");

        assert!(!reply.actions.is_empty(), "expected at least one tool call, got none");
        let lowered = reply.text.to_lowercase();
        assert!(
            lowered.contains("oct") || lowered.contains("14") || lowered.contains("venue")
                || lowered.contains("book"),
            "answer should reflect Marco's message, got: {}",
            reply.text
        );
    }

    #[tokio::test]
    #[ignore = "requires local Ollama"]
    async fn live_assistant_draft_reply_lands_in_drafts_and_is_not_sent() {
        let state = seeded_state();
        let reply = run_chat(
            &state,
            "s-draft",
            "Draft a short reply to Priya confirming the PO number is unchanged.",
        )
        .await
        .expect("assistant replies");

        let (count, status): (i64, Option<String>) = {
            let conn = state.db.lock().unwrap();
            let count = conn
                .query_row("SELECT count(*) FROM drafts", [], |r| r.get(0))
                .unwrap();
            let status = conn
                .query_row("SELECT status FROM drafts LIMIT 1", [], |r| r.get(0))
                .ok();
            (count, status)
        };
        assert_eq!(count, 1, "exactly one draft should have been created");
        assert_eq!(status.as_deref(), Some("pending_review"));
        assert!(
            reply.text.to_lowercase().contains("draft"),
            "assistant should tell the user the reply is a draft, got: {}",
            reply.text
        );

        // Nothing in the assistant path may mark a draft sent.
        let sent: i64 = state
            .db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM drafts WHERE status = 'sent'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sent, 0);
    }

    #[tokio::test]
    #[ignore = "requires local Ollama"]
    async fn live_assistant_persists_turns_to_history() {
        let state = seeded_state();
        run_chat(&state, "s-hist", "List the newest message from Priya.")
            .await
            .unwrap();
        let roles: Vec<String> = {
            let conn = state.db.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT role FROM chat_messages WHERE session_id='s-hist' ORDER BY id")
                .unwrap();
            stmt.query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(roles.first().map(String::as_str), Some("user"));
        assert_eq!(roles.last().map(String::as_str), Some("assistant"));
    }
}
