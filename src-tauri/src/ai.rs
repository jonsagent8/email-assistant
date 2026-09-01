//! Local-AI operations over cached mail: per-message summaries and inbox triage.
//! Both read/write the SQLite cache and call the local model via [`crate::llm`].

use crate::llm;
use crate::state::AppState;
use serde::Serialize;
use serde_json::Value;

/// A single cached message, enough to reason about without another fetch.
pub struct EmailRow {
    pub id: i64,
    pub from_addr: String,
    pub subject: String,
    pub date: String,
    pub body_text: String,
}

pub fn load_email(state: &AppState, email_id: i64) -> Result<EmailRow, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT id, from_addr, subject, date, body_text FROM emails WHERE id = ?1",
        [email_id],
        |row| {
            Ok(EmailRow {
                id: row.get(0)?,
                from_addr: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                subject: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                date: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                body_text: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => format!("no cached email with id {email_id}"),
        other => other.to_string(),
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}

// ---------- summaries ----------

/// Returns the cached summary if one exists, otherwise generates, stores, and
/// returns a fresh one.
pub async fn summarize_email(state: &AppState, email_id: i64) -> Result<String, String> {
    if let Ok(conn) = state.db.lock() {
        if let Ok(existing) = conn.query_row(
            "SELECT summary_text FROM summaries WHERE email_id = ?1",
            [email_id],
            |row| row.get::<_, String>(0),
        ) {
            return Ok(existing);
        }
    }

    let email = load_email(state, email_id)?;
    let model = state.setting("chat_model", "qwen3:8b-q4_K_M");
    let prompt = format!(
        "Summarize this email in 2-3 sentences. Be concrete about what the sender wants or is \
         telling the recipient. Do not add a preamble.\n\n\
         From: {}\nSubject: {}\nDate: {}\n\n{}",
        email.from_addr,
        email.subject,
        email.date,
        truncate(&email.body_text, 6000),
    );

    let summary = llm::generate(&state.http, &model, &prompt).await?.trim().to_string();

    if let Ok(conn) = state.db.lock() {
        let _ = conn.execute(
            "INSERT INTO summaries (email_id, model_name, summary_text) VALUES (?1, ?2, ?3)
             ON CONFLICT(email_id) DO UPDATE SET
                summary_text = excluded.summary_text,
                model_name = excluded.model_name,
                generated_at = datetime('now')",
            rusqlite::params![email_id, model, summary],
        );
    }
    Ok(summary)
}

// ---------- triage ----------

#[derive(Serialize)]
pub struct TriageResult {
    pub triaged: usize,
}

const CATEGORIES: &[&str] = &["urgent", "needs_reply", "fyi", "newsletter", "spam"];

fn extract_json_object(s: &str) -> Option<Value> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    serde_json::from_str(&s[start..=end]).ok()
}

/// Classifies up to `limit` of the most recent not-yet-triaged INBOX messages
/// for `account_id`. Returns how many were updated.
pub async fn triage(state: &AppState, account_id: i64, limit: i64) -> Result<TriageResult, String> {
    let pending: Vec<EmailRow> = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, from_addr, subject, date, body_text
                 FROM emails
                 WHERE account_id = ?1 AND folder = 'INBOX' AND triaged_at IS NULL
                 ORDER BY uid DESC LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![account_id, limit], |row| {
                Ok(EmailRow {
                    id: row.get(0)?,
                    from_addr: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    subject: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    date: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    body_text: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?
    };

    let model = state.setting("triage_model", "qwen3:1.7b");
    let mut triaged = 0usize;

    for email in pending {
        let prompt = format!(
            "You are triaging an email. Reply with ONLY a JSON object, no prose, shaped exactly:\n\
             {{\"category\": one of {:?}, \"labels\": [short lowercase tags], \
             \"action_items\": [things the recipient must do, empty if none]}}\n\n\
             From: {}\nSubject: {}\nDate: {}\n\n{}",
            CATEGORIES,
            email.from_addr,
            email.subject,
            email.date,
            truncate(&email.body_text, 4000),
        );

        let raw = match llm::generate(&state.http, &model, &prompt).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("triage of email {} failed: {e}", email.id);
                continue;
            }
        };

        let (category, labels, action_items) = match extract_json_object(&raw) {
            Some(v) => {
                let category = v
                    .get("category")
                    .and_then(|c| c.as_str())
                    .filter(|c| CATEGORIES.contains(c))
                    .unwrap_or("fyi")
                    .to_string();
                let labels = v.get("labels").cloned().unwrap_or(Value::Array(vec![]));
                let action_items =
                    v.get("action_items").cloned().unwrap_or(Value::Array(vec![]));
                (category, labels.to_string(), action_items.to_string())
            }
            None => ("fyi".to_string(), "[]".to_string(), "[]".to_string()),
        };

        if let Ok(conn) = state.db.lock() {
            let _ = conn.execute(
                "UPDATE emails SET category = ?1, labels = ?2, action_items = ?3,
                    triaged_at = datetime('now') WHERE id = ?4",
                rusqlite::params![category, labels, action_items, email.id],
            );
        }
        triaged += 1;
    }

    Ok(TriageResult { triaged })
}

/// Pulls `(category, labels_json, action_items_json)` out of a model's raw triage
/// reply, defaulting anything missing or out-of-vocabulary to a safe `fyi`.
/// Split out from [`triage`] so it can be tested without a live model.
#[cfg(test)]
fn parse_triage(raw: &str) -> (String, String, String) {
    match extract_json_object(raw) {
        Some(v) => {
            let category = v
                .get("category")
                .and_then(|c| c.as_str())
                .filter(|c| CATEGORIES.contains(c))
                .unwrap_or("fyi")
                .to_string();
            let labels = v.get("labels").cloned().unwrap_or(Value::Array(vec![]));
            let action_items = v.get("action_items").cloned().unwrap_or(Value::Array(vec![]));
            (category, labels.to_string(), action_items.to_string())
        }
        None => ("fyi".to_string(), "[]".to_string(), "[]".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_object_pulls_a_bare_object() {
        let v = extract_json_object(r#"{"category":"urgent"}"#).unwrap();
        assert_eq!(v.get("category").unwrap(), "urgent");
    }

    #[test]
    fn extract_json_object_ignores_prose_around_the_object() {
        let v = extract_json_object("Sure! Here you go:\n{\"category\":\"fyi\"}\nHope that helps")
            .unwrap();
        assert_eq!(v.get("category").unwrap(), "fyi");
    }

    #[test]
    fn extract_json_object_spans_from_first_brace_to_last() {
        let v = extract_json_object(r#"noise {"a":{"b":1}} tail"#).unwrap();
        assert_eq!(v.get("a").unwrap().get("b").unwrap(), 1);
    }

    #[test]
    fn extract_json_object_returns_none_on_garbage() {
        assert!(extract_json_object("no json here").is_none());
        assert!(extract_json_object("{not valid json}").is_none());
    }

    #[test]
    fn truncate_leaves_short_strings_alone() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn truncate_cuts_on_char_boundaries_and_adds_ellipsis() {
        assert_eq!(truncate("hello world", 5), "hello…");
        // multi-byte: must not panic slicing mid-codepoint
        assert_eq!(truncate("héllo wörld", 4), "héll…");
    }

    #[test]
    fn parse_triage_keeps_a_valid_category() {
        let (cat, labels, actions) = parse_triage(
            r#"{"category":"needs_reply","labels":["invoice"],"action_items":["pay it"]}"#,
        );
        assert_eq!(cat, "needs_reply");
        assert!(labels.contains("invoice"));
        assert!(actions.contains("pay it"));
    }

    #[test]
    fn parse_triage_downgrades_an_unknown_category_to_fyi() {
        let (cat, ..) = parse_triage(r#"{"category":"SUPER_URGENT","labels":[]}"#);
        assert_eq!(cat, "fyi");
    }

    #[test]
    fn parse_triage_defaults_missing_fields_to_empty_arrays() {
        let (cat, labels, actions) = parse_triage(r#"{"category":"spam"}"#);
        assert_eq!(cat, "spam");
        assert_eq!(labels, "[]");
        assert_eq!(actions, "[]");
    }

    #[test]
    fn parse_triage_falls_back_entirely_when_there_is_no_json() {
        let (cat, labels, actions) = parse_triage("the model rambled and produced nothing useful");
        assert_eq!((cat.as_str(), labels.as_str(), actions.as_str()), ("fyi", "[]", "[]"));
    }

    // ---- live tests: require a local Ollama with the default models.
    // Run with:  cargo test -- --ignored --test-threads=1

    use crate::state::AppState;

    fn seeded_state() -> AppState {
        let conn = crate::db::memory_db();
        conn.execute(
            "INSERT INTO accounts (id, email, imap_host, imap_port, smtp_host, smtp_port)
             VALUES (1, 'me@example.com', 'imap.example.com', 993, 'smtp.example.com', 587)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails (id, account_id, folder, uid, from_addr, subject, date, snippet, body_text)
             VALUES (1, 1, 'INBOX', 101, 'Dana Ruiz <dana@acme.test>',
                     'Contract signature needed today',
                     '2026-08-30T09:00:00+00:00',
                     'Please sign the attached contract by 5pm.',
                     'Hi, the client moved the deadline up. We need your signature on the renewal contract by 5pm today or we lose the slot. Reply once it is done. Thanks, Dana')",
            [],
        )
        .unwrap();
        AppState::new(conn)
    }

    #[tokio::test]
    #[ignore = "requires local Ollama"]
    async fn live_triage_classifies_an_urgent_action_email() {
        let state = seeded_state();
        let result = triage(&state, 1, 5).await.expect("triage runs");
        assert_eq!(result.triaged, 1);

        let conn = state.db.lock().unwrap();
        let (category, action_items): (String, String) = conn
            .query_row(
                "SELECT category, action_items FROM emails WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(
            CATEGORIES.contains(&category.as_str()),
            "category {category:?} not in vocabulary"
        );
        assert!(
            matches!(category.as_str(), "urgent" | "needs_reply"),
            "a same-day signature request should be urgent/needs_reply, got {category:?}"
        );
        assert!(action_items.starts_with('['), "action_items should be a JSON array");
    }

    #[tokio::test]
    #[ignore = "requires local Ollama"]
    async fn live_triage_is_idempotent_second_pass_finds_nothing() {
        let state = seeded_state();
        assert_eq!(triage(&state, 1, 5).await.unwrap().triaged, 1);
        assert_eq!(
            triage(&state, 1, 5).await.unwrap().triaged,
            0,
            "already-triaged messages must be skipped"
        );
    }

    #[tokio::test]
    #[ignore = "requires local Ollama"]
    async fn live_summary_is_cached_after_first_generation() {
        let state = seeded_state();
        let first = summarize_email(&state, 1).await.expect("summary generates");
        assert!(!first.trim().is_empty());

        // Second call must return the stored row, not regenerate.
        let stored: String = state
            .db
            .lock()
            .unwrap()
            .query_row("SELECT summary_text FROM summaries WHERE email_id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, first);
        let second = summarize_email(&state, 1).await.unwrap();
        assert_eq!(second, first);
    }
}
