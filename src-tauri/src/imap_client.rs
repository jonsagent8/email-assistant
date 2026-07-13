use mail_parser::MessageParser;
use rustls_pki_types::ServerName;
use std::sync::Arc;
use tokio_rustls::TlsConnector;

/// Known IMAP/SMTP settings for common providers, keyed by email domain, so the
/// account-setup UI never has to ask a non-technical user for server hostnames/ports.
pub struct ProviderPreset {
    pub imap_host: &'static str,
    pub imap_port: u16,
    pub smtp_host: &'static str,
    pub smtp_port: u16,
    /// Deep link to the provider's app-password / security settings page.
    pub app_password_url: &'static str,
}

pub fn detect_provider(email: &str) -> Option<ProviderPreset> {
    let domain = email.rsplit('@').next()?.to_lowercase();
    match domain.as_str() {
        "gmail.com" | "googlemail.com" => Some(ProviderPreset {
            imap_host: "imap.gmail.com",
            imap_port: 993,
            smtp_host: "smtp.gmail.com",
            smtp_port: 587,
            app_password_url: "https://myaccount.google.com/apppasswords",
        }),
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" => Some(ProviderPreset {
            imap_host: "outlook.office365.com",
            imap_port: 993,
            smtp_host: "smtp.office365.com",
            smtp_port: 587,
            app_password_url: "https://account.live.com/proofs/AppPassword",
        }),
        "yahoo.com" | "ymail.com" => Some(ProviderPreset {
            imap_host: "imap.mail.yahoo.com",
            imap_port: 993,
            smtp_host: "smtp.mail.yahoo.com",
            smtp_port: 587,
            app_password_url: "https://login.yahoo.com/account/security",
        }),
        "icloud.com" | "me.com" | "mac.com" => Some(ProviderPreset {
            imap_host: "imap.mail.me.com",
            imap_port: 993,
            smtp_host: "smtp.mail.me.com",
            smtp_port: 587,
            app_password_url: "https://appleid.apple.com/account/manage",
        }),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct FetchedEmail {
    pub uid: u32,
    pub message_id: Option<String>,
    pub from_addr: Option<String>,
    pub to_addr: Option<String>,
    pub subject: Option<String>,
    pub date: Option<String>,
    pub snippet: String,
    pub body_text: String,
}

fn address_to_string(addr: Option<&mail_parser::Address>) -> Option<String> {
    let addr = addr?;
    addr.first().and_then(|a| {
        a.address()
            .map(|s| s.to_string())
            .or_else(|| a.name().map(|s| s.to_string()))
    })
}

async fn connect_tls(
    host: &str,
    port: u16,
) -> Result<tokio_rustls::client::TlsStream<tokio::net::TcpStream>, String> {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));

    let tcp = tokio::net::TcpStream::connect((host, port))
        .await
        .map_err(|e| format!("could not reach {host}:{port}: {e}"))?;
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|e| format!("invalid host name {host}: {e}"))?;
    connector
        .connect(server_name, tcp)
        .await
        .map_err(|e| format!("TLS handshake with {host} failed: {e}"))
}

/// Connects, logs in, and fetches the most recent `limit` messages from INBOX.
/// Used both to validate a new account (small limit) and for regular sync.
pub async fn fetch_recent_inbox(
    imap_host: &str,
    imap_port: u16,
    email: &str,
    password: &str,
    limit: u32,
) -> Result<Vec<FetchedEmail>, String> {
    let tls_stream = connect_tls(imap_host, imap_port).await?;
    let client = async_imap::Client::new(tls_stream);

    let mut session = client
        .login(email, password)
        .await
        .map_err(|(e, _client)| format!("login failed: {e}"))?;

    let mailbox = session
        .select("INBOX")
        .await
        .map_err(|e| format!("could not open INBOX: {e}"))?;

    if mailbox.exists == 0 {
        session.logout().await.ok();
        return Ok(Vec::new());
    }

    let start = mailbox.exists.saturating_sub(limit.saturating_sub(1)).max(1);
    let seq_set = format!("{start}:{}", mailbox.exists);

    let parser = MessageParser::default();
    let mut results = Vec::new();
    {
        use futures::TryStreamExt;
        let mut stream = session
            .fetch(&seq_set, "(UID BODY.PEEK[])")
            .await
            .map_err(|e| format!("fetch failed: {e}"))?;

        while let Some(fetch) = stream
            .try_next()
            .await
            .map_err(|e| format!("fetch stream error: {e}"))?
        {
            let Some(uid) = fetch.uid else { continue };
            let Some(raw) = fetch.body() else { continue };
            let Some(message) = parser.parse(raw) else {
                continue;
            };

            let body_text = message
                .body_text(0)
                .map(|c| c.to_string())
                .unwrap_or_default();
            let snippet: String = body_text.chars().take(200).collect();

            results.push(FetchedEmail {
                uid,
                message_id: message.message_id().map(|s| s.to_string()),
                from_addr: address_to_string(message.from()),
                to_addr: address_to_string(message.to()),
                subject: message.subject().map(|s| s.to_string()),
                date: message.date().map(|d| d.to_rfc3339()),
                snippet,
                body_text,
            });
        }
    }

    session.logout().await.ok();
    results.sort_by(|a, b| b.uid.cmp(&a.uid));
    Ok(results)
}
