use crate::state::AppState;
use crate::{credentials, imap_client};
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct ProviderInfo {
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub app_password_url: String,
}

/// Looks up known IMAP/SMTP settings from the email's domain so the setup form
/// can skip asking for server hostnames entirely for common providers.
#[tauri::command]
pub fn detect_provider(email: String) -> Option<ProviderInfo> {
    imap_client::detect_provider(&email).map(|p| ProviderInfo {
        imap_host: p.imap_host.to_string(),
        imap_port: p.imap_port,
        smtp_host: p.smtp_host.to_string(),
        smtp_port: p.smtp_port,
        app_password_url: p.app_password_url.to_string(),
    })
}

#[derive(Serialize, Clone)]
pub struct AccountInfo {
    pub id: i64,
    pub email: String,
    pub display_name: Option<String>,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
}

/// Validates the credentials actually work (real IMAP login + fetch) before
/// persisting anything, then stores the password only in the OS keychain —
/// the SQLite row never contains a secret.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // one arg per account field; the setup form sends them flat
pub async fn add_account(
    state: State<'_, AppState>,
    email: String,
    password: String,
    imap_host: String,
    imap_port: u16,
    smtp_host: String,
    smtp_port: u16,
    display_name: Option<String>,
) -> Result<AccountInfo, String> {
    imap_client::fetch_recent_inbox(&imap_host, imap_port, &email, &password, 1).await?;

    credentials::store_password(&email, "imap", &password)?;
    credentials::store_password(&email, "smtp", &password)?;

    let id = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO accounts (email, display_name, imap_host, imap_port, smtp_host, smtp_port)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![email, display_name, imap_host, imap_port, smtp_host, smtp_port],
        )
        .map_err(|e| e.to_string())?;
        conn.last_insert_rowid()
    };

    Ok(AccountInfo {
        id,
        email,
        display_name,
        imap_host,
        imap_port,
        smtp_host,
        smtp_port,
    })
}

#[tauri::command]
pub fn list_accounts(state: State<'_, AppState>) -> Result<Vec<AccountInfo>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, email, display_name, imap_host, imap_port, smtp_host, smtp_port
             FROM accounts ORDER BY id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(AccountInfo {
                id: row.get(0)?,
                email: row.get(1)?,
                display_name: row.get(2)?,
                imap_host: row.get(3)?,
                imap_port: row.get(4)?,
                smtp_host: row.get(5)?,
                smtp_port: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}
