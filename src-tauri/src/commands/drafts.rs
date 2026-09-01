//! Reply drafts: generate with the local model, review/edit in the UI, and — only
//! on an explicit user action — send over SMTP.

use crate::state::AppState;
use crate::{credentials, llm, smtp};
use serde::Serialize;
use tauri::State;

#[derive(Serialize, Clone)]
pub struct DraftInfo {
    pub id: i64,
    pub email_id: i64,
    /// Who the reply will go to (the original sender).
    pub to: String,
    pub subject: String,
    pub draft_text: String,
    pub status: String,
    pub created_at: String,
    /// Context for the review panel.
    pub original_from: String,
    pub original_subject: String,
}

struct ReplyContext {
    account_id: i64,
    from_addr: String,
    subject: String,
    body_text: String,
    message_id: Option<String>,
}

fn load_reply_context(state: &AppState, email_id: i64) -> Result<ReplyContext, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT account_id, from_addr, subject, body_text, message_id
         FROM emails WHERE id = ?1",
        [email_id],
        |row| {
            Ok(ReplyContext {
                account_id: row.get(0)?,
                from_addr: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                subject: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                body_text: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                message_id: row.get(4)?,
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => format!("no cached email with id {email_id}"),
        other => other.to_string(),
    })
}

fn reply_subject(original: &str) -> String {
    if original.trim_start().to_lowercase().starts_with("re:") {
        original.to_string()
    } else {
        format!("Re: {original}")
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}

/// Generates a reply draft for `email_id` and stores it as `pending_review`.
#[tauri::command]
pub async fn generate_draft(
    state: State<'_, AppState>,
    email_id: i64,
    instructions: Option<String>,
) -> Result<DraftInfo, String> {
    generate_draft_internal(&state, email_id, instructions).await
}

/// Shared implementation, callable from the assistant's tool loop.
pub async fn generate_draft_internal(
    state: &AppState,
    email_id: i64,
    instructions: Option<String>,
) -> Result<DraftInfo, String> {
    llm::ensure_running(state).await?;
    let ctx = load_reply_context(state, email_id)?;

    let account_email: String = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT email FROM accounts WHERE id = ?1",
            [ctx.account_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?
    };

    let model = state.setting("chat_model", "qwen3:8b-q4_K_M");
    let steer = instructions
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Write a courteous, concise reply that moves the conversation forward.");

    let prompt = format!(
        "Write the body of an email reply. Output only the reply body — no subject line, no \
         \"Here is\" preamble, no surrounding quotes. Sign off as the account owner ({account_email}).\n\n\
         Instructions: {steer}\n\n\
         --- Original message ---\nFrom: {}\nSubject: {}\n\n{}",
        ctx.from_addr,
        ctx.subject,
        truncate(&ctx.body_text, 6000),
    );

    let draft_text = llm::generate(&state.http, &model, &prompt).await?.trim().to_string();
    let subject = reply_subject(&ctx.subject);

    let id = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO drafts (email_id, draft_text, status) VALUES (?1, ?2, 'pending_review')",
            rusqlite::params![email_id, draft_text],
        )
        .map_err(|e| e.to_string())?;
        conn.last_insert_rowid()
    };

    Ok(DraftInfo {
        id,
        email_id,
        to: ctx.from_addr.clone(),
        subject,
        draft_text,
        status: "pending_review".into(),
        created_at: String::new(),
        original_from: ctx.from_addr,
        original_subject: ctx.subject,
    })
}

#[tauri::command]
pub fn list_drafts(state: State<'_, AppState>) -> Result<Vec<DraftInfo>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT d.id, d.email_id, d.draft_text, d.status, d.created_at,
                    e.from_addr, e.subject
             FROM drafts d JOIN emails e ON e.id = d.email_id
             WHERE d.status IN ('pending_review', 'approved')
             ORDER BY d.created_at DESC, d.id DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            let original_subject: Option<String> = row.get(6)?;
            Ok(DraftInfo {
                id: row.get(0)?,
                email_id: row.get(1)?,
                draft_text: row.get(2)?,
                status: row.get(3)?,
                created_at: row.get(4)?,
                original_from: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                to: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                subject: reply_subject(original_subject.as_deref().unwrap_or("")),
                original_subject: original_subject.unwrap_or_default(),
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_draft(state: State<'_, AppState>, draft_id: i64, text: String) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let changed = conn
        .execute(
            "UPDATE drafts SET draft_text = ?1 WHERE id = ?2 AND status = 'pending_review'",
            rusqlite::params![text, draft_id],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("draft not found or already sent".into());
    }
    Ok(())
}

#[tauri::command]
pub fn discard_draft(state: State<'_, AppState>, draft_id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE drafts SET status = 'discarded' WHERE id = ?1",
        [draft_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Sends a pending draft. This is the only path that actually transmits mail and
/// is called strictly from a confirmed user action in the UI.
#[tauri::command]
pub async fn send_draft(state: State<'_, AppState>, draft_id: i64) -> Result<(), String> {
    let (email_id, draft_text, status) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT email_id, draft_text, status FROM drafts WHERE id = ?1",
            [draft_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => "draft not found".to_string(),
            other => other.to_string(),
        })?
    };

    if status == "sent" {
        return Err("this draft was already sent".into());
    }

    let ctx = load_reply_context(&state, email_id)?;

    let (account_email, smtp_host, smtp_port, display_name) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT email, smtp_host, smtp_port, display_name FROM accounts WHERE id = ?1",
            [ctx.account_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u16>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?
    };

    let password = credentials::get_password(&account_email, "smtp")?;

    smtp::send_reply(smtp::OutgoingReply {
        smtp_host: &smtp_host,
        smtp_port,
        from_email: &account_email,
        from_name: display_name.as_deref(),
        password: &password,
        to: &ctx.from_addr,
        subject: &reply_subject(&ctx.subject),
        body: &draft_text,
        in_reply_to: ctx.message_id.clone(),
    })
    .await?;

    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE drafts SET status = 'sent', sent_at = datetime('now') WHERE id = ?1",
        [draft_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{reply_subject, truncate};

    #[test]
    fn reply_subject_prefixes_re_once() {
        assert_eq!(reply_subject("Lunch?"), "Re: Lunch?");
    }

    #[test]
    fn reply_subject_does_not_double_prefix() {
        assert_eq!(reply_subject("Re: Lunch?"), "Re: Lunch?");
        assert_eq!(reply_subject("RE: Lunch?"), "RE: Lunch?");
        assert_eq!(reply_subject("re: lunch?"), "re: lunch?");
    }

    #[test]
    fn reply_subject_ignores_leading_whitespace_when_detecting_re() {
        assert_eq!(reply_subject("   Re: threaded"), "   Re: threaded");
    }

    #[test]
    fn reply_subject_treats_res_prefix_as_not_a_reply() {
        // "Response to..." starts with "res" but not "re:" — must still get prefixed
        assert_eq!(reply_subject("Response needed"), "Re: Response needed");
    }

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("abcdef", 3), "abc…");
        assert_eq!(truncate("çödé", 2), "çö…");
    }
}
