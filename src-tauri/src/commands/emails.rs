use crate::state::AppState;
use crate::{credentials, imap_client};
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
}

fn query_cached_emails(conn: &Connection, account_id: i64) -> Result<Vec<EmailInfo>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, uid, from_addr, to_addr, subject, date, snippet, is_read
             FROM emails WHERE account_id = ?1 AND folder = 'INBOX' ORDER BY uid DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![account_id], |row| {
            Ok(EmailInfo {
                id: row.get(0)?,
                uid: row.get(1)?,
                from_addr: row.get(2)?,
                to_addr: row.get(3)?,
                subject: row.get(4)?,
                date: row.get(5)?,
                snippet: row.get(6)?,
                is_read: row.get::<_, i64>(7)? != 0,
            })
        })
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
