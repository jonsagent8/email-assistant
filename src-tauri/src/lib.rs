mod ai;
mod commands;
mod credentials;
mod db;
mod imap_client;
mod llm;
mod smtp;
mod state;

use state::AppState;
use tauri::{Manager, RunEvent};

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
            app.manage(AppState::new(conn));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::accounts::detect_provider,
            commands::accounts::add_account,
            commands::accounts::list_accounts,
            commands::emails::get_cached_emails,
            commands::emails::get_email_full,
            commands::emails::search_emails,
            commands::emails::sync_inbox,
            commands::emails::triage_inbox,
            commands::emails::summarize_email,
            commands::assistant::assistant_chat,
            commands::drafts::generate_draft,
            commands::drafts::list_drafts,
            commands::drafts::update_draft,
            commands::drafts::discard_draft,
            commands::drafts::send_draft,
            commands::settings::get_setting,
            commands::settings::set_setting,
            commands::settings::ai_status,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                if let Some(state) = app.try_state::<AppState>() {
                    llm::shutdown(&state);
                }
            }
        });
}
