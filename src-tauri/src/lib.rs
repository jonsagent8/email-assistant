mod commands;
mod credentials;
mod db;
mod imap_client;
mod state;

use state::AppState;
use std::sync::Mutex;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Both `ring` and `aws-lc-rs` end up compiled in transitively (rustls's own
    // default features vs. what we request), which makes rustls refuse to guess
    // a default crypto provider at runtime. Pick one explicitly, once, up front.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            let conn = db::init_db(&app_data_dir)?;
            app.manage(AppState {
                db: Mutex::new(conn),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::accounts::detect_provider,
            commands::accounts::add_account,
            commands::accounts::list_accounts,
            commands::emails::get_cached_emails,
            commands::emails::sync_inbox,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
