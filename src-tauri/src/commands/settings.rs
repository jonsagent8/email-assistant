//! Key/value settings and local-AI status for the Settings view.

use crate::llm;
use crate::state::AppState;
use serde::Serialize;
use tauri::State;

#[tauri::command]
pub fn get_setting(state: State<'_, AppState>, key: String) -> Result<Option<String>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    match conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        [key],
        |row| row.get::<_, String>(0),
    ) {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub fn set_setting(state: State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Serialize)]
pub struct AiStatus {
    pub running: bool,
    pub models: Vec<String>,
    pub chat_model: String,
    pub triage_model: String,
}

/// Starts the local AI server if needed, then reports what's available.
#[tauri::command]
pub async fn ai_status(state: State<'_, AppState>) -> Result<AiStatus, String> {
    let running = llm::ensure_running(&state).await.is_ok();
    let models = if running {
        llm::list_models(&state.http).await.unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(AiStatus {
        running,
        models,
        chat_model: state.setting("chat_model", "qwen3:8b-q4_K_M"),
        triage_model: state.setting("triage_model", "qwen3:1.7b"),
    })
}
