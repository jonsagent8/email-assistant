//! Outgoing mail over SMTP (STARTTLS) via `lettre`. Only ever invoked from an
//! explicit user "Send" action — there is no automatic sending anywhere.

use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

pub struct OutgoingReply<'a> {
    pub smtp_host: &'a str,
    pub smtp_port: u16,
    pub from_email: &'a str,
    pub from_name: Option<&'a str>,
    pub password: &'a str,
    pub to: &'a str,
    /// Subject already normalised (e.g. prefixed with "Re: ").
    pub subject: &'a str,
    pub body: &'a str,
    /// RFC Message-ID of the message being replied to, angle brackets included.
    pub in_reply_to: Option<String>,
}

pub async fn send_reply(reply: OutgoingReply<'_>) -> Result<(), String> {
    let from_mbox = match reply.from_name {
        Some(name) => format!("{name} <{}>", reply.from_email),
        None => reply.from_email.to_string(),
    };

    let mut builder = Message::builder()
        .from(from_mbox.parse().map_err(|e| format!("bad From address: {e}"))?)
        .to(reply.to.parse().map_err(|e| format!("bad To address: {e}"))?)
        .subject(reply.subject)
        .header(ContentType::TEXT_PLAIN);

    if let Some(mid) = &reply.in_reply_to {
        builder = builder.in_reply_to(mid.clone()).references(mid.clone());
    }

    let email = builder
        .body(reply.body.to_string())
        .map_err(|e| format!("could not build message: {e}"))?;

    let creds = Credentials::new(reply.from_email.to_string(), reply.password.to_string());
    let mailer: AsyncSmtpTransport<Tokio1Executor> =
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(reply.smtp_host)
            .map_err(|e| format!("SMTP setup failed: {e}"))?
            .port(reply.smtp_port)
            .credentials(creds)
            .build();

    mailer
        .send(email)
        .await
        .map_err(|e| format!("sending failed: {e}"))?;
    Ok(())
}
