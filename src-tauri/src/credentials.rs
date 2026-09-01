use keyring::Entry;

const SERVICE: &str = "com.emailassistant.app";

/// Never allow an empty username to reach the keyring — on macOS an empty
/// service/username is treated as a wildcard lookup, which would silently
/// match the wrong credential.
fn entry(email: &str, purpose: &str) -> Result<Entry, String> {
    if email.trim().is_empty() {
        return Err("email must not be empty".into());
    }
    let username = format!("{email}:{purpose}");
    Entry::new(SERVICE, &username).map_err(|e| e.to_string())
}

pub fn store_password(email: &str, purpose: &str, password: &str) -> Result<(), String> {
    entry(email, purpose)?
        .set_password(password)
        .map_err(|e| e.to_string())
}

pub fn get_password(email: &str, purpose: &str) -> Result<String, String> {
    entry(email, purpose)?
        .get_password()
        .map_err(|e| e.to_string())
}

/// Kept for the not-yet-built "remove account" flow, which must clear both the
/// `imap` and `smtp` keychain entries.
#[allow(dead_code)]
pub fn delete_password(email: &str, purpose: &str) -> Result<(), String> {
    entry(email, purpose)?
        .delete_credential()
        .map_err(|e| e.to_string())
}
