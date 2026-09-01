use crate::state::AppState;
use crate::{ai, credentials, imap_client};
use rusqlite::Connection;
use serde::Serialize;
use tauri::State;

#[derive(Serialize, Clone)]
pub struct EmailInfo {
    pub id: i64,
    pub uid: i64,
    pub from_addr: Option<String>,
    pub to_addr: Option<String>,
    pub subject: Option<String>,
    pub date: Option<String>,
    pub snippet: String,
    pub is_read: bool,
    pub category: Option<String>,
    /// JSON array strings, passed straight through to the frontend.
    pub labels: Option<String>,
    pub action_items: Option<String>,
}

const EMAIL_COLUMNS: &str =
    "id, uid, from_addr, to_addr, subject, date, snippet, is_read, category, labels, action_items";

fn row_to_email(row: &rusqlite::Row) -> rusqlite::Result<EmailInfo> {
    Ok(EmailInfo {
        id: row.get(0)?,
        uid: row.get(1)?,
        from_addr: row.get(2)?,
        to_addr: row.get(3)?,
        subject: row.get(4)?,
        date: row.get(5)?,
        snippet: row.get(6)?,
        is_read: row.get::<_, i64>(7)? != 0,
        category: row.get(8)?,
        labels: row.get(9)?,
        action_items: row.get(10)?,
    })
}

fn query_cached_emails(conn: &Connection, account_id: i64) -> Result<Vec<EmailInfo>, String> {
    let sql = format!(
        "SELECT {EMAIL_COLUMNS} FROM emails
         WHERE account_id = ?1 AND folder = 'INBOX' ORDER BY uid DESC"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![account_id], row_to_email)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Loads whatever is already cached locally — no network — so the app has
/// something to show instantly on startup before a sync completes.
#[tauri::command]
pub fn get_cached_emails(
    state: State<'_, AppState>,
    account_id: i64,
) -> Result<Vec<EmailInfo>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    query_cached_emails(&conn, account_id)
}

/// Full text of one cached message. Used by the assistant's `get_email` tool and
/// by the "Draft reply" flow.
#[tauri::command]
pub fn get_email_full(state: State<'_, AppState>, email_id: i64) -> Result<String, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT body_text FROM emails WHERE id = ?1",
        [email_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .map(|b| b.unwrap_or_default())
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => format!("no cached email with id {email_id}"),
        other => other.to_string(),
    })
}

/// Substring search across the local cache. Any of the filters may be omitted.
/// `since` is an ISO date/datetime string compared lexically against the stored
/// RFC3339 `date`.
#[tauri::command]
pub fn search_emails(
    state: State<'_, AppState>,
    query: Option<String>,
    from: Option<String>,
    since: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<EmailInfo>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let limit = limit.unwrap_or(20).clamp(1, 100);

    let mut sql = format!(
        "SELECT {EMAIL_COLUMNS} FROM emails WHERE folder = 'INBOX'"
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(q) = query.as_ref().filter(|q| !q.trim().is_empty()) {
        sql.push_str(" AND (subject LIKE ? OR body_text LIKE ? OR from_addr LIKE ?)");
        let like = format!("%{q}%");
        params.push(Box::new(like.clone()));
        params.push(Box::new(like.clone()));
        params.push(Box::new(like));
    }
    if let Some(f) = from.as_ref().filter(|f| !f.trim().is_empty()) {
        sql.push_str(" AND from_addr LIKE ?");
        params.push(Box::new(format!("%{f}%")));
    }
    if let Some(s) = since.as_ref().filter(|s| !s.trim().is_empty()) {
        sql.push_str(" AND date >= ?");
        params.push(Box::new(s.clone()));
    }
    sql.push_str(" ORDER BY date DESC, uid DESC LIMIT ?");
    params.push(Box::new(limit));

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), row_to_email)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Connects over IMAP, fetches recent INBOX messages, upserts them into the
/// local cache, and returns the merged cached list.
#[tauri::command]
pub async fn sync_inbox(
    state: State<'_, AppState>,
    account_id: i64,
) -> Result<Vec<EmailInfo>, String> {
    let (email, imap_host, imap_port) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT email, imap_host, imap_port FROM accounts WHERE id = ?1",
            rusqlite::params![account_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u16>(2)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?
    };

    let password = credentials::get_password(&email, "imap")?;
    let fetched = imap_client::fetch_recent_inbox(&imap_host, imap_port, &email, &password, 50).await?;

    let conn = state.db.lock().map_err(|e| e.to_string())?;
    for msg in &fetched {
        conn.execute(
            "INSERT INTO emails (account_id, folder, uid, message_id, from_addr, to_addr, subject, date, snippet, body_text, has_attachments)
             VALUES (?1, 'INBOX', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)
             ON CONFLICT(account_id, folder, uid) DO UPDATE SET
                subject = excluded.subject,
                snippet = excluded.snippet,
                body_text = excluded.body_text",
            rusqlite::params![
                account_id,
                msg.uid,
                msg.message_id,
                msg.from_addr,
                msg.to_addr,
                msg.subject,
                msg.date,
                msg.snippet,
                msg.body_text
            ],
        )
        .map_err(|e| e.to_string())?;
    }

    query_cached_emails(&conn, account_id)
}

/// AI-classifies recent un-triaged INBOX messages, then returns the refreshed list.
#[tauri::command]
pub async fn triage_inbox(
    state: State<'_, AppState>,
    account_id: i64,
    limit: Option<i64>,
) -> Result<Vec<EmailInfo>, String> {
    crate::llm::ensure_running(&state).await?;
    ai::triage(&state, account_id, limit.unwrap_or(20).clamp(1, 50)).await?;
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    query_cached_emails(&conn, account_id)
}

/// Summarize a single message (cached after first run).
#[tauri::command]
pub async fn summarize_email(
    state: State<'_, AppState>,
    email_id: i64,
) -> Result<String, String> {
    crate::llm::ensure_running(&state).await?;
    ai::summarize_email(&state, email_id).await
}
