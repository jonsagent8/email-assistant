use rusqlite::Connection;
use std::fs;
use std::path::Path;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS accounts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email TEXT NOT NULL UNIQUE,
    display_name TEXT,
    imap_host TEXT NOT NULL,
    imap_port INTEGER NOT NULL,
    smtp_host TEXT NOT NULL,
    smtp_port INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS emails (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder TEXT NOT NULL,
    uid INTEGER NOT NULL,
    message_id TEXT,
    from_addr TEXT,
    to_addr TEXT,
    subject TEXT,
    date TEXT,
    snippet TEXT,
    body_text TEXT,
    has_attachments INTEGER NOT NULL DEFAULT 0,
    is_read INTEGER NOT NULL DEFAULT 0,
    fetched_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(account_id, folder, uid)
);

CREATE TABLE IF NOT EXISTS summaries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email_id INTEGER NOT NULL REFERENCES emails(id) ON DELETE CASCADE,
    model_name TEXT NOT NULL,
    summary_text TEXT NOT NULL,
    generated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(email_id)
);

CREATE TABLE IF NOT EXISTS drafts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email_id INTEGER NOT NULL REFERENCES emails(id) ON DELETE CASCADE,
    draft_text TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending_review','approved','sent','discarded')) DEFAULT 'pending_review',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    sent_at TEXT
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO settings (key, value) VALUES ('auto_send_enabled', 'false');
INSERT OR IGNORE INTO settings (key, value) VALUES ('poll_interval_seconds', '300');
INSERT OR IGNORE INTO settings (key, value) VALUES ('chat_model', 'qwen3:8b-q4_K_M');
INSERT OR IGNORE INTO settings (key, value) VALUES ('triage_model', 'qwen3:1.7b');

CREATE TABLE IF NOT EXISTS chat_sessions (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS chat_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    tool_trace TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
"#;

/// Columns added to `emails` after the initial schema shipped. Applied with
/// `ALTER TABLE ... ADD COLUMN`, which errors if the column already exists — that
/// error is expected on every launch after the first and is swallowed.
const EMAIL_COLUMN_MIGRATIONS: &[&str] = &[
    "ALTER TABLE emails ADD COLUMN category TEXT",
    "ALTER TABLE emails ADD COLUMN labels TEXT",
    "ALTER TABLE emails ADD COLUMN action_items TEXT",
    "ALTER TABLE emails ADD COLUMN triaged_at TEXT",
];

pub fn init_db(app_data_dir: &Path) -> rusqlite::Result<Connection> {
    fs::create_dir_all(app_data_dir).expect("failed to create app data dir");
    let db_path = app_data_dir.join("email-assistant.sqlite3");
    let conn = Connection::open(db_path)?;
    prepare(&conn)?;
    Ok(conn)
}

/// Applies the pragmas, base schema, and column migrations to an open
/// connection. Split from [`init_db`] so tests can drive it against an
/// in-memory database.
fn prepare(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    conn.execute_batch(SCHEMA)?;
    migrate(conn);
    Ok(())
}

fn migrate(conn: &Connection) {
    for stmt in EMAIL_COLUMN_MIGRATIONS {
        match conn.execute(stmt, []) {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(_, Some(msg))) if msg.contains("duplicate column") => {}
            Err(e) => eprintln!("migration `{stmt}` failed: {e}"),
        }
    }
}

/// An in-memory database with the full schema applied — for tests in this and
/// other modules that need a real `Connection` without touching disk.
#[cfg(test)]
pub(crate) fn memory_db() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    prepare(&conn).expect("apply schema");
    conn
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Connection {
        memory_db()
    }

    #[test]
    fn schema_creates_every_expected_table() {
        let conn = fresh();
        let mut names: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        names.retain(|n| !n.starts_with("sqlite_"));
        assert_eq!(
            names,
            [
                "accounts", "chat_messages", "chat_sessions", "drafts",
                "emails", "settings", "summaries",
            ]
        );
    }

    #[test]
    fn triage_columns_are_added_by_migration() {
        let conn = fresh();
        let cols: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('emails')")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for c in ["category", "labels", "action_items", "triaged_at"] {
            assert!(cols.contains(&c.to_string()), "emails.{c} missing");
        }
    }

    #[test]
    fn prepare_is_idempotent() {
        let conn = fresh();
        // Second application must not error (schema is IF NOT EXISTS; migrations
        // swallow "duplicate column").
        prepare(&conn).unwrap();
        prepare(&conn).unwrap();
    }

    #[test]
    fn default_settings_are_seeded_including_the_no_auto_send_guardrail() {
        let conn = fresh();
        let auto_send: String = conn
            .query_row("SELECT value FROM settings WHERE key='auto_send_enabled'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(auto_send, "false");
        let chat: String = conn
            .query_row("SELECT value FROM settings WHERE key='chat_model'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(chat, "qwen3:8b-q4_K_M");
    }

    #[test]
    fn drafts_status_check_rejects_unknown_states() {
        let conn = fresh();
        conn.execute(
            "INSERT INTO accounts (email, imap_host, imap_port, smtp_host, smtp_port)
             VALUES ('a@b.com','h',993,'h',587)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails (account_id, folder, uid) VALUES (1,'INBOX',1)",
            [],
        )
        .unwrap();
        let ok = conn.execute(
            "INSERT INTO drafts (email_id, draft_text, status) VALUES (1,'hi','pending_review')",
            [],
        );
        assert!(ok.is_ok());
        let bad = conn.execute(
            "INSERT INTO drafts (email_id, draft_text, status) VALUES (1,'hi','auto_sent')",
            [],
        );
        assert!(bad.is_err(), "CHECK constraint should reject 'auto_sent'");
    }

    #[test]
    fn deleting_an_account_cascades_to_its_emails() {
        let conn = fresh();
        conn.execute(
            "INSERT INTO accounts (email, imap_host, imap_port, smtp_host, smtp_port)
             VALUES ('a@b.com','h',993,'h',587)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO emails (account_id, folder, uid) VALUES (1,'INBOX',1)", [])
            .unwrap();
        conn.execute("DELETE FROM accounts WHERE id=1", []).unwrap();
        let remaining: i64 = conn
            .query_row("SELECT count(*) FROM emails", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }
}
