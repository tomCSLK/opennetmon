//! Outgoing e-mail: settings, validation and the SMTP client. User-facing messages are catalog messages.

#[cfg(test)]
pub(crate) mod testsmtp;

use std::time::Duration;

use lettre::message::header::ContentType;
use lettre::message::{Mailbox, MessageBuilder};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::credentials::Secret;
use crate::devices::validate_host;
use crate::i18n::Msg;

pub const MAX_RECIPIENTS: usize = 20;
const MAX_FIELD_CHARS: usize = 256;
const SMTP_TIMEOUT: Duration = Duration::from_secs(15);
/// Upper bound for one complete delivery (connect, TLS, auth, transfer).
const SEND_DEADLINE: Duration = Duration::from_secs(40);
const MAX_ERROR_CHARS: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// Unencrypted; only for a trusted local relay.
    None,
    /// Plain connection upgraded with STARTTLS (usually port 587).
    StartTls,
    /// TLS from the first byte (usually port 465).
    Tls,
}

impl Security {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::StartTls => "starttls",
            Self::Tls => "tls",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "none" => Self::None,
            "starttls" => Self::StartTls,
            "tls" => Self::Tls,
            _ => return None,
        })
    }

    pub fn label_id(self) -> &'static str {
        match self {
            Self::None => "mail-security-none",
            Self::StartTls => "mail-security-starttls",
            Self::Tls => "mail-security-tls",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Self::None => 25,
            Self::StartTls => 587,
            Self::Tls => 465,
        }
    }
}

/// What the settings form submits (texts as typed).
#[derive(Debug, Clone)]
pub struct MailInput {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: String,
    /// `None`: keep the stored password.
    pub password: Option<Secret>,
    pub from_address: String,
    /// Addresses separated by newlines, commas or semicolons.
    pub recipients: String,
    pub notify_warnings: bool,
}

/// Validated input, ready to store.
#[derive(Debug, Clone)]
pub struct CheckedMail {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: Option<String>,
    pub password: Option<Secret>,
    pub from_address: String,
    pub recipients: Vec<String>,
    pub notify_warnings: bool,
}

/// Splits and validates a recipient list; returns the normalized addresses.
pub fn parse_recipients(text: &str) -> Result<Vec<String>, Msg> {
    let mut out: Vec<String> = Vec::new();
    for part in text.split(['\n', '\r', ',', ';']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let address = parse_address(part)?;
        if !out.contains(&address) {
            out.push(address);
        }
    }
    if out.len() > MAX_RECIPIENTS {
        return Err(Msg::new("mail-too-many-recipients").with("count", MAX_RECIPIENTS));
    }
    Ok(out)
}

/// A plain address like `name@example.org` (no display name, no angle brackets).
pub fn parse_address(text: &str) -> Result<String, Msg> {
    let text = text.trim();
    if text.chars().count() > MAX_FIELD_CHARS
        || text.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(Msg::new("mail-address-invalid").with("address", shorten(text)));
    }
    text.parse::<Address>()
        .map(|a| a.to_string())
        .map_err(|_| Msg::new("mail-address-invalid").with("address", shorten(text)))
}

fn shorten(text: &str) -> String {
    text.chars().take(40).filter(|c| !c.is_control()).collect()
}

impl MailInput {
    /// Checks the input. With notifications switched on, server, sender and at least one
    /// recipient are required; switched off, anything half-finished may be saved.
    pub fn check(&self) -> Result<CheckedMail, Msg> {
        let host = self.host.trim();
        let host = if host.is_empty() {
            String::new()
        } else {
            validate_host(host)?
        };
        if self.port == 0 {
            return Err(Msg::new("mail-port-invalid"));
        }
        let username = self.username.trim();
        if username.chars().count() > MAX_FIELD_CHARS || username.chars().any(char::is_control) {
            return Err(Msg::new("mail-username-invalid"));
        }
        if let Some(p) = &self.password
            && (p.expose().chars().count() > MAX_FIELD_CHARS
                || p.expose().chars().any(char::is_control))
        {
            return Err(Msg::new("mail-password-invalid"));
        }
        let from_address = match self.from_address.trim() {
            "" => String::new(),
            text => parse_address(text)?,
        };
        let recipients = parse_recipients(&self.recipients)?;
        if self.enabled {
            if host.is_empty() {
                return Err(Msg::new("mail-host-missing"));
            }
            if from_address.is_empty() {
                return Err(Msg::new("mail-from-missing"));
            }
            if recipients.is_empty() {
                return Err(Msg::new("mail-recipients-missing"));
            }
        }
        Ok(CheckedMail {
            enabled: self.enabled,
            host,
            port: self.port,
            security: self.security,
            username: (!username.is_empty()).then(|| username.to_owned()),
            password: self.password.clone(),
            from_address,
            recipients,
            notify_warnings: self.notify_warnings,
        })
    }
}

/// Everything needed to send, with the password decrypted. Never log or display it.
#[derive(Clone)]
pub struct MailConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: Option<String>,
    pub password: Option<Secret>,
    pub from_address: String,
    pub recipients: Vec<String>,
    pub notify_warnings: bool,
}

impl std::fmt::Debug for MailConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MailConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("{0}")]
    Config(Msg),
    /// Delivery failed; the text comes from the SMTP client (or is a timeout).
    #[error("{0}")]
    Send(Msg),
}

impl MailError {
    /// Explanation for the UI; contains no secrets.
    pub fn user_message(&self) -> Msg {
        match self {
            Self::Config(m) => m.clone(),
            Self::Send(m) => Msg::new("mail-send-failed").with("detail", m.clone()),
        }
    }
}

fn build_message(config: &MailConfig, subject: &str, body: &str) -> Result<Message, MailError> {
    let bad = |id: &'static str| MailError::Config(Msg::new(id));
    let from: Mailbox = config
        .from_address
        .parse::<Address>()
        .map(|a| Mailbox::new(Some("opennetmon".to_owned()), a))
        .map_err(|_| bad("mail-from-invalid"))?;
    let mut builder: MessageBuilder = Message::builder().from(from).subject(subject);
    if config.recipients.is_empty() {
        return Err(MailError::Config(Msg::new("mail-no-recipients")));
    }
    for recipient in &config.recipients {
        let address = recipient
            .parse::<Address>()
            .map_err(|_| bad("mail-recipient-invalid"))?;
        builder = builder.to(Mailbox::new(None, address));
    }
    builder
        .header(ContentType::TEXT_PLAIN)
        .body(body.to_owned())
        .map_err(|e| {
            MailError::Config(Msg::new("mail-message-invalid").with("detail", e.to_string()))
        })
}

fn transport(config: &MailConfig) -> Result<AsyncSmtpTransport<Tokio1Executor>, MailError> {
    let host = config.host.as_str();
    let unusable = |e: lettre::transport::smtp::Error| {
        MailError::Config(Msg::new("mail-server-unusable").with("detail", e.to_string()))
    };
    let builder = match config.security {
        Security::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host),
        Security::StartTls => {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host).map_err(unusable)?
        }
        Security::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(host).map_err(unusable)?,
    };
    let mut builder = builder.port(config.port).timeout(Some(SMTP_TIMEOUT));
    if let (Some(user), Some(password)) = (&config.username, &config.password) {
        builder = builder.credentials(Credentials::new(user.clone(), password.expose().to_owned()));
    }
    Ok(builder.build())
}

fn describe(error: impl std::fmt::Display) -> String {
    let text = error.to_string();
    if text.chars().count() > MAX_ERROR_CHARS {
        text.chars().take(MAX_ERROR_CHARS - 1).collect::<String>() + "…"
    } else {
        text
    }
}

/// Sends one plain-text mail to all recipients.
pub async fn send(config: &MailConfig, subject: &str, body: &str) -> Result<(), MailError> {
    if config.host.is_empty() {
        return Err(MailError::Config(Msg::new("mail-no-server")));
    }
    let message = build_message(config, subject, body)?;
    let transport = transport(config)?;
    match tokio::time::timeout(SEND_DEADLINE, transport.send(message)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(MailError::Send(Msg::Text(describe(e)))),
        Err(_) => Err(MailError::Send(Msg::new("mail-timeout"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testsmtp::{Behavior, TestSmtp};

    fn input() -> MailInput {
        MailInput {
            enabled: true,
            host: "mail.example.org".to_owned(),
            port: 587,
            security: Security::StartTls,
            username: String::new(),
            password: None,
            from_address: "opennetmon@example.org".to_owned(),
            recipients: "a@example.org".to_owned(),
            notify_warnings: true,
        }
    }

    fn config(port: u16) -> MailConfig {
        MailConfig {
            enabled: true,
            host: "127.0.0.1".to_owned(),
            port,
            security: Security::None,
            username: None,
            password: None,
            from_address: "opennetmon@example.org".to_owned(),
            recipients: vec!["admin@example.org".to_owned(), "ops@example.org".to_owned()],
            notify_warnings: true,
        }
    }

    #[test]
    fn recipients_are_split_validated_and_deduplicated() {
        let list =
            parse_recipients("a@example.org, b@example.org;\n a@example.org\r\n\n c@example.org")
                .unwrap();
        assert_eq!(list, ["a@example.org", "b@example.org", "c@example.org"]);
        assert!(parse_recipients("").unwrap().is_empty());
        for bad in [
            "nope",
            "a@",
            "@b.org",
            "a b@example.org",
            "Name <a@example.org>",
            "a@example.org\nBcc: x@y.org, x",
        ] {
            assert!(parse_recipients(bad).is_err(), "{bad:?}");
        }
        let many = (0..=MAX_RECIPIENTS)
            .map(|i| format!("u{i}@example.org"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_recipients(&many).is_err());
    }

    #[test]
    fn enabled_settings_need_server_sender_and_recipient() {
        assert!(input().check().is_ok());
        for (field, message) in [
            ("host", "SMTP-Server"),
            ("from", "Absenderadresse"),
            ("to", "Empfänger"),
        ] {
            let mut i = input();
            match field {
                "host" => i.host.clear(),
                "from" => i.from_address.clear(),
                _ => i.recipients.clear(),
            }
            let err = i.check().unwrap_err();
            assert!(err.to_string().contains(message), "{field}: {err}");
            i.enabled = false;
            assert!(
                i.check().is_ok(),
                "half-finished settings may be saved while disabled"
            );
        }
    }

    #[test]
    fn invalid_fields_are_rejected_with_a_reason() {
        let mut i = input();
        i.host = "bad host!".to_owned();
        assert!(i.check().is_err());
        let mut i = input();
        i.port = 0;
        assert!(i.check().unwrap_err().to_string().contains("Port"));
        let mut i = input();
        i.from_address = "nope".to_owned();
        assert!(
            i.check()
                .unwrap_err()
                .to_string()
                .contains("keine gültige E-Mail-Adresse")
        );
        let mut i = input();
        i.username = "u\nx".to_owned();
        assert!(i.check().is_err());
        let mut i = input();
        i.password = Some(Secret::new("a\u{0007}b"));
        assert!(i.check().is_err());
    }

    #[test]
    fn username_and_host_are_normalized() {
        let mut i = input();
        i.host = " Mail.Example.ORG ".to_owned();
        i.username = "  ".to_owned();
        let checked = i.check().unwrap();
        assert_eq!(checked.host, "mail.example.org");
        assert_eq!(checked.username, None);
    }

    #[test]
    fn security_names_round_trip() {
        for s in [Security::None, Security::StartTls, Security::Tls] {
            assert_eq!(Security::parse(s.as_str()), Some(s));
        }
        assert_eq!(Security::parse("ssl"), None);
        assert_eq!(Security::Tls.default_port(), 465);
    }

    #[test]
    fn debug_output_hides_the_password() {
        let mut c = config(25);
        c.username = Some("user".to_owned());
        c.password = Some(Secret::new("geheim-123"));
        assert!(!format!("{c:?}").contains("geheim-123"));
    }

    #[tokio::test]
    async fn a_mail_reaches_every_recipient_with_subject_and_body() {
        let smtp = TestSmtp::spawn(Behavior::default()).await;
        send(
            &config(smtp.port),
            "[opennetmon] Ausfall: Test",
            "Zeile 1\n.Zeile mit Punkt\nÄÖÜ",
        )
        .await
        .unwrap();
        let mails = smtp.received();
        assert_eq!(mails.len(), 1);
        assert_eq!(mails[0].from, "opennetmon@example.org");
        assert_eq!(mails[0].to, ["admin@example.org", "ops@example.org"]);
        assert!(
            mails[0]
                .data
                .contains("Subject: [opennetmon] Ausfall: Test"),
            "{}",
            mails[0].data
        );
        assert!(
            mails[0]
                .data
                .contains("From: opennetmon <opennetmon@example.org>"),
            "{}",
            mails[0].data
        );
    }

    #[tokio::test]
    async fn credentials_are_sent_and_wrong_ones_are_reported() {
        let behavior = Behavior {
            require_auth: Some(("user".to_owned(), "pw-123".to_owned())),
            ..Behavior::default()
        };
        let smtp = TestSmtp::spawn(behavior).await;
        let mut c = config(smtp.port);
        c.username = Some("user".to_owned());
        c.password = Some(Secret::new("pw-123"));
        send(&c, "s", "b").await.unwrap();
        assert_eq!(smtp.received()[0].auth_user.as_deref(), Some("user"));

        c.password = Some(Secret::new("wrong-pw"));
        let err = send(&c, "s", "b").await.unwrap_err();
        assert!(matches!(err, MailError::Send(_)), "{err:?}");
        assert!(!err.user_message().to_string().contains("wrong-pw"));
        assert_eq!(smtp.received().len(), 1);
    }

    #[tokio::test]
    async fn rejected_recipients_and_dead_servers_are_errors_not_panics() {
        let smtp = TestSmtp::spawn(Behavior {
            reject_recipients: true,
            ..Behavior::default()
        })
        .await;
        let err = send(&config(smtp.port), "s", "b").await.unwrap_err();
        assert!(
            err.user_message()
                .to_string()
                .starts_with("Versand fehlgeschlagen"),
            "{err:?}"
        );

        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        assert!(matches!(
            send(&config(port), "s", "b").await,
            Err(MailError::Send(_))
        ));
    }

    #[tokio::test]
    async fn incomplete_settings_fail_with_a_config_error() {
        let mut c = config(25);
        c.host.clear();
        assert!(matches!(
            send(&c, "s", "b").await,
            Err(MailError::Config(_))
        ));
        let mut c = config(25);
        c.recipients.clear();
        assert!(matches!(
            send(&c, "s", "b").await,
            Err(MailError::Config(_))
        ));
    }
}
