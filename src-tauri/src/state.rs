use rusqlite::Connection;
use std::sync::Mutex;
use tokio::process::Child;

pub struct AppState {
    pub db: Mutex<Connection>,
    /// Shared HTTP client for talking to the local Ollama server.
    pub http: reqwest::Client,
    /// The `ollama serve` process we started, if it wasn't already running.
    /// Killed on app exit so we don't leave an orphaned server behind.
    pub ollama_child: Mutex<Option<Child>>,
}

impl AppState {
    pub fn new(db: Connection) -> Self {
        AppState {
            db: Mutex::new(db),
            http: reqwest::Client::new(),
            ollama_child: Mutex::new(None),
        }
    }

    /// Reads a row from the `settings` table, falling back to `default` when the
    /// key is missing.
    pub fn setting(&self, key: &str, default: &str) -> String {
        let conn = match self.db.lock() {
            Ok(c) => c,
            Err(_) => return default.to_string(),
        };
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [key],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|_| default.to_string())
    }
}
