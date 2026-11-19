//! Outgoing e-mail: settings, validation and the SMTP client. User-facing messages are catalog messages.
//!
//! A [`Mail`] is a plain-text part and, optionally, an HTML part with inline images (`cid:`); it goes
//! out as `multipart/alternative` (text, then `multipart/related` with the HTML and its images).

pub mod testmail;
#[cfg(test)]
pub(crate) mod testsmtp;
pub mod view;

use std::time::Duration;

use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MessageBuilder, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::credentials::Secret;
use crate::devices::validate_host;
use crate::i18n::{Lang, Msg};

pub const MAX_RECIPIENTS: usize = 20;
const MAX_FIELD_CHARS: usize = 256;
const SMTP_TIMEOUT: Duration = Duration::from_secs(15);
/// Upper bound for one complete delivery (connect, TLS, auth, transfer).
const SEND_DEADLINE: Duration = Duration::from_secs(40);
const MAX_ERROR_CHARS: usize = 300;
/// Longest accepted address of the web interface.
pub const MAX_UI_URL_CHARS: usize = 200;

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
    /// Language of the mails, the same for all recipients.
    pub language: Lang,
    /// Address of the web interface for links in mails, as typed (empty = none).
    pub ui_url: String,
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
    pub language: Lang,
    /// Normalized by [`parse_ui_url`].
    pub ui_url: Option<String>,
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

/// The address of the web interface: an absolute `http://` or `https://` URL with a host, an
/// optional port and an optional path, without user info, query or fragment. Returned with the
/// scheme and host in lower case and without a trailing slash; empty input means "none". Links in
/// mails are built from it plus fixed paths and numeric ids only.
pub fn parse_ui_url(text: &str) -> Result<Option<String>, Msg> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let invalid = || Msg::new("mail-ui-url-invalid");
    if text.chars().count() > MAX_UI_URL_CHARS || !text.is_ascii() {
        return Err(invalid());
    }
    let lower = text.to_ascii_lowercase();
    let (scheme, rest) = if lower.starts_with("https://") {
        ("https", &text[8..])
    } else if lower.starts_with("http://") {
        ("http", &text[7..])
    } else {
        return Err(invalid());
    };
    let (authority, path) = rest.find('/').map_or((rest, ""), |i| rest.split_at(i));
    if authority.contains('@') {
        return Err(invalid());
    }
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        // IPv6 literal: [2001:db8::1] or [2001:db8::1]:8443
        let (ip, after) = inner.split_once(']').ok_or_else(invalid)?;
        let ip: std::net::Ipv6Addr = ip.parse().map_err(|_| invalid())?;
        (format!("[{ip}]"), after.strip_prefix(':'))
    } else {
        let (host, port) = match authority.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        };
        (validate_host(host).map_err(|_| invalid())?, port)
    };
    let port = match port {
        None => String::new(),
        Some(p) => {
            if p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid());
            }
            match p.parse::<u16>() {
                Ok(n) if n > 0 => format!(":{n}"),
                _ => return Err(invalid()),
            }
        }
    };
    let path_ok = path
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-._~/%".contains(&b));
    if !path_ok || path.contains("//") || path.split('/').any(|seg| seg == "..") {
        return Err(invalid());
    }
    let path = path.trim_end_matches('/');
    Ok(Some(format!("{scheme}://{host}{port}{path}")))
}

/// Text for a mail header: control characters (CR, LF, tabs …) become spaces, runs of white space
/// one space, and the result is cut to `max` characters (with "…").
pub fn header_text(text: &str, max: usize) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let joined = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() > max {
        let cut: String = joined.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", cut.trim_end())
    } else {
        joined
    }
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
        let ui_url = parse_ui_url(&self.ui_url)?;
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
            language: self.language,
            ui_url,
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
    pub language: Lang,
    /// Address of the web interface for buttons and links (see [`parse_ui_url`]); `None` = no links.
    pub ui_url: Option<String>,
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

/// An image the HTML part shows with `<img src="cid:…">`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineImage {
    /// Without angle brackets, e.g. `logo@opennetmon`.
    pub content_id: String,
    /// e.g. `image/png`.
    pub content_type: &'static str,
    pub data: Vec<u8>,
}

/// A complete mail: subject, plain text and optionally HTML with its inline images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    pub subject: String,
    pub text: String,
    /// `None`: a plain-text mail.
    pub html: Option<String>,
    pub inline: Vec<InlineImage>,
}

impl Mail {
    /// A mail with only a text part.
    pub fn plain(subject: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            text: text.into(),
            html: None,
            inline: Vec::new(),
        }
    }
}

fn build_message(config: &MailConfig, mail: &Mail) -> Result<Message, MailError> {
    let bad = |id: &'static str| MailError::Config(Msg::new(id));
    let invalid = |e: String| MailError::Config(Msg::new("mail-message-invalid").with("detail", e));
    let from: Mailbox = config
        .from_address
        .parse::<Address>()
        .map(|a| Mailbox::new(Some("opennetmon".to_owned()), a))
        .map_err(|_| bad("mail-from-invalid"))?;
    let subject = header_text(&mail.subject, 250);
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
    let Some(html) = &mail.html else {
        return builder
            .header(ContentType::TEXT_PLAIN)
            .body(mail.text.clone())
            .map_err(|e| invalid(e.to_string()));
    };
    let html_part = SinglePart::html(html.clone());
    let body = MultiPart::alternative().singlepart(SinglePart::plain(mail.text.clone()));
    let body = if mail.inline.is_empty() {
        body.singlepart(html_part)
    } else {
        let mut related = MultiPart::related().singlepart(html_part);
        for image in &mail.inline {
            let content_type =
                ContentType::parse(image.content_type).map_err(|e| invalid(e.to_string()))?;
            related = related.singlepart(
                Attachment::new_inline(image.content_id.clone())
                    .body(image.data.clone(), content_type),
            );
        }
        body.multipart(related)
    };
    builder.multipart(body).map_err(|e| invalid(e.to_string()))
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

/// Sends one mail to all recipients.
pub async fn send(config: &MailConfig, mail: &Mail) -> Result<(), MailError> {
    if config.host.is_empty() {
        return Err(MailError::Config(Msg::new("mail-no-server")));
    }
    let message = build_message(config, mail)?;
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
            language: Lang::De,
            ui_url: String::new(),
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
            language: Lang::De,
            ui_url: None,
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
            &Mail::plain(
                "[opennetmon] Ausfall: Test",
                "Zeile 1\n.Zeile mit Punkt\nÄÖÜ",
            ),
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
        send(&c, &Mail::plain("s", "b")).await.unwrap();
        assert_eq!(smtp.received()[0].auth_user.as_deref(), Some("user"));

        c.password = Some(Secret::new("wrong-pw"));
        let err = send(&c, &Mail::plain("s", "b")).await.unwrap_err();
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
        let err = send(&config(smtp.port), &Mail::plain("s", "b"))
            .await
            .unwrap_err();
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
            send(&config(port), &Mail::plain("s", "b")).await,
            Err(MailError::Send(_))
        ));
    }

    #[tokio::test]
    async fn incomplete_settings_fail_with_a_config_error() {
        let mut c = config(25);
        c.host.clear();
        assert!(matches!(
            send(&c, &Mail::plain("s", "b")).await,
            Err(MailError::Config(_))
        ));
        let mut c = config(25);
        c.recipients.clear();
        assert!(matches!(
            send(&c, &Mail::plain("s", "b")).await,
            Err(MailError::Config(_))
        ));
    }

    #[test]
    fn the_ui_address_is_checked_and_normalised() {
        let ok = |text: &str| parse_ui_url(text).unwrap();
        assert_eq!(ok(""), None);
        assert_eq!(ok("   "), None);
        assert_eq!(
            ok("https://opennetmon.example.org/").as_deref(),
            Some("https://opennetmon.example.org")
        );
        assert_eq!(
            ok("HTTP://192.0.2.5:8443/monitor/").as_deref(),
            Some("http://192.0.2.5:8443/monitor")
        );
        assert_eq!(
            ok("https://[2001:db8::1]:8443").as_deref(),
            Some("https://[2001:db8::1]:8443")
        );
        for bad in [
            "opennetmon.example.org",
            "ftp://example.org",
            "javascript:alert(1)",
            "https://user:pw@example.org",
            "https://example.org/?a=1",
            "https://example.org/#top",
            "https://example.org:0",
            "https://example.org:70000",
            "https://example.org:http",
            "https://exa mple.org",
            "https://example.org/a//b",
            "https://example.org/../x",
            "https://example.org/\"><script>",
            "https://bücher.example",
            "https://[::1",
        ] {
            assert_eq!(
                parse_ui_url(bad),
                Err(Msg::new("mail-ui-url-invalid")),
                "{bad}"
            );
        }
        let long = format!("https://example.org/{}", "a".repeat(MAX_UI_URL_CHARS));
        assert!(parse_ui_url(&long).is_err());
    }

    #[test]
    fn header_text_has_no_line_breaks_and_a_limit() {
        assert_eq!(
            header_text("Ausfall\r\nBcc: x@example.org\t ok", 100),
            "Ausfall Bcc: x@example.org ok"
        );
        assert_eq!(header_text("\u{7}a\u{1b}b", 100), "a b");
        assert_eq!(header_text("abcdefghij", 5), "abcd…");
        assert_eq!(header_text("abc", 5), "abc");
    }

    #[tokio::test]
    async fn html_mails_carry_text_html_and_the_inline_logo() {
        let smtp = TestSmtp::spawn(Behavior::default()).await;
        let mail = Mail {
            subject: "[opennetmon] Test\r\nBcc: evil@example.org".to_owned(),
            text: "Klartext".to_owned(),
            html: Some(format!(
                "<html><body><img src=\"cid:{}\">HTML-Teil</body></html>",
                view::LOGO_CID
            )),
            inline: vec![view::logo()],
        };
        send(&config(smtp.port), &mail).await.unwrap();
        let received = &smtp.received()[0];
        let data = &received.data;
        assert!(
            data.contains("Content-Type: multipart/alternative"),
            "{data}"
        );
        assert!(data.contains("Content-Type: multipart/related"), "{data}");
        assert!(data.contains("Content-ID: <logo@opennetmon>"), "{data}");
        assert!(data.contains("Content-Disposition: inline"), "{data}");
        assert!(data.contains("Content-Type: image/png"), "{data}");
        assert_eq!(
            received.subject(),
            "[opennetmon] Test Bcc: evil@example.org"
        );
        assert!(!data.contains("\nBcc:"), "no injected header");
        assert_eq!(received.to, ["admin@example.org", "ops@example.org"]);
        assert_eq!(received.part("text/plain").as_deref(), Some("Klartext"));
        assert!(
            received
                .part("text/html")
                .is_some_and(|h| h.contains("HTML-Teil") && h.contains("cid:logo@opennetmon"))
        );

        // Without images, no related part.
        let smtp = TestSmtp::spawn(Behavior::default()).await;
        let mail = Mail {
            inline: Vec::new(),
            ..mail
        };
        send(&config(smtp.port), &mail).await.unwrap();
        let data = &smtp.received()[0].data;
        assert!(data.contains("multipart/alternative") && !data.contains("multipart/related"));
    }
}
