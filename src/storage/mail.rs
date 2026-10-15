//! Persistence of the e-mail settings (one row) and the notification log.

use sqlx::{FromRow, SqlitePool};

use crate::credentials::Secret;
use crate::crypto::SecretBox;
use crate::error::{Error, Result};
use crate::mail::{CheckedMail, MailConfig, Security};

const PASSWORD_CONTEXT: &str = "smtp.password";
const MAX_LOGGED_TEXT: usize = 300;

/// Settings as shown in the UI: the password only as "set / not set".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailView {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: String,
    pub password_set: bool,
    pub from_address: String,
    /// One address per line, as the form shows them.
    pub recipients: String,
    pub notify_warnings: bool,
}

#[derive(FromRow)]
struct Row {
    enabled: i64,
    host: String,
    port: i64,
    security: String,
    username: Option<String>,
    password_enc: Option<Vec<u8>>,
    from_address: String,
    recipients: String,
    notify_warnings: i64,
}

async fn fetch(pool: &SqlitePool) -> Result<Row> {
    Ok(sqlx::query_as::<_, Row>(
        "SELECT enabled, host, port, security, username, password_enc, from_address, recipients, \
         notify_warnings FROM mail_settings WHERE id = 1",
    )
    .fetch_one(pool)
    .await?)
}

fn security(row: &Row) -> Result<Security> {
    Security::parse(&row.security)
        .ok_or_else(|| Error::Internal("invalid mail security in database".to_owned()))
}

fn port(row: &Row) -> Result<u16> {
    u16::try_from(row.port).map_err(|_| Error::Internal("invalid mail port in database".to_owned()))
}

pub async fn get_view(pool: &SqlitePool) -> Result<MailView> {
    let row = fetch(pool).await?;
    Ok(MailView {
        enabled: row.enabled != 0,
        port: port(&row)?,
        security: security(&row)?,
        username: row.username.clone().unwrap_or_default(),
        password_set: row.password_enc.is_some(),
        recipients: row.recipients.replace(',', "\n"),
        notify_warnings: row.notify_warnings != 0,
        host: row.host,
        from_address: row.from_address,
    })
}

/// The settings with the decrypted password, for sending.
pub async fn load_config(pool: &SqlitePool, secrets: &SecretBox) -> Result<MailConfig> {
    let row = fetch(pool).await?;
    let password = row
        .password_enc
        .as_deref()
        .map(|blob| secrets.decrypt(blob, PASSWORD_CONTEXT).map(Secret::new))
        .transpose()?;
    Ok(MailConfig {
        enabled: row.enabled != 0,
        port: port(&row)?,
        security: security(&row)?,
        recipients: row
            .recipients
            .split(',')
            .filter(|r| !r.is_empty())
            .map(str::to_owned)
            .collect(),
        notify_warnings: row.notify_warnings != 0,
        username: row.username,
        password,
        host: row.host,
        from_address: row.from_address,
    })
}

/// Stores validated settings. A missing password keeps the stored one, unless the user name is
/// empty (no authentication, so no password).
pub async fn save(pool: &SqlitePool, secrets: &SecretBox, input: &CheckedMail) -> Result<()> {
    let current = fetch(pool).await?;
    let password_enc = match (&input.username, &input.password) {
        (None, _) => None,
        (Some(_), Some(password)) => Some(secrets.encrypt(password.expose(), PASSWORD_CONTEXT)?),
        (Some(_), None) => current.password_enc,
    };
    sqlx::query(
        "UPDATE mail_settings SET enabled = ?, host = ?, port = ?, security = ?, username = ?, \
         password_enc = ?, from_address = ?, recipients = ?, notify_warnings = ?, \
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = 1",
    )
    .bind(i64::from(input.enabled))
    .bind(&input.host)
    .bind(i64::from(input.port))
    .bind(input.security.as_str())
    .bind(&input.username)
    .bind(password_enc)
    .bind(&input.from_address)
    .bind(input.recipients.join(","))
    .bind(i64::from(input.notify_warnings))
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Alert,
    Test,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Alert => "alert",
            Self::Test => "test",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Alert => "Alarm",
            Self::Test => "Test",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub ts_ms: i64,
    pub kind: Kind,
    pub subject: String,
    pub recipients: String,
    pub ok: bool,
    pub error: String,
}

fn clip(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_LOGGED_TEXT)
        .collect()
}

/// Records the outcome of a delivery attempt (`error` is `None` on success).
pub async fn record_notification(
    pool: &SqlitePool,
    ts_ms: i64,
    kind: Kind,
    subject: &str,
    recipients: &[String],
    error: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO notifications (ts_ms, kind, subject, recipients, ok, error) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(ts_ms)
    .bind(kind.as_str())
    .bind(clip(subject))
    .bind(clip(&recipients.join(", ")))
    .bind(i64::from(error.is_none()))
    .bind(error.map(clip))
    .execute(pool)
    .await?;
    Ok(())
}

/// The newest delivery attempts first.
pub async fn recent_notifications(pool: &SqlitePool, limit: u32) -> Result<Vec<Notification>> {
    type R = (i64, String, String, String, i64, Option<String>);
    let rows: Vec<R> = sqlx::query_as(
        "SELECT ts_ms, kind, subject, recipients, ok, error FROM notifications \
         ORDER BY ts_ms DESC, id DESC LIMIT ?",
    )
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(ts_ms, kind, subject, recipients, ok, error)| Notification {
                ts_ms,
                kind: if kind == "test" {
                    Kind::Test
                } else {
                    Kind::Alert
                },
                subject,
                recipients,
                ok: ok != 0,
                error: error.unwrap_or_default(),
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::MailInput;
    use crate::storage::{connect, migrate};

    async fn db() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("m.db")).await.unwrap();
        migrate(&pool).await.unwrap();
        (dir, pool)
    }

    fn input(username: &str, password: Option<&str>) -> CheckedMail {
        MailInput {
            enabled: true,
            host: "mail.example.org".to_owned(),
            port: 587,
            security: Security::StartTls,
            username: username.to_owned(),
            password: password.map(Secret::new),
            from_address: "netmon@example.org".to_owned(),
            recipients: "a@example.org\nb@example.org".to_owned(),
            notify_warnings: false,
        }
        .check()
        .unwrap()
    }

    #[tokio::test]
    async fn defaults_are_disabled_and_empty() {
        let (_dir, pool) = db().await;
        let view = get_view(&pool).await.unwrap();
        assert!(!view.enabled && !view.password_set && view.host.is_empty());
        assert_eq!(
            (view.port, view.security, view.notify_warnings),
            (587, Security::StartTls, true)
        );
    }

    #[tokio::test]
    async fn settings_round_trip_and_the_password_is_encrypted() {
        let (_dir, pool) = db().await;
        let secrets = SecretBox::from_key([3u8; 32]);
        save(&pool, &secrets, &input("netmon", Some("pw-geheim-1")))
            .await
            .unwrap();

        let view = get_view(&pool).await.unwrap();
        assert_eq!(view.username, "netmon");
        assert!(view.password_set);
        assert_eq!(view.recipients, "a@example.org\nb@example.org");
        assert!(!view.notify_warnings);

        let config = load_config(&pool, &secrets).await.unwrap();
        assert_eq!(
            config.password.as_ref().map(Secret::expose),
            Some("pw-geheim-1")
        );
        assert_eq!(config.recipients, ["a@example.org", "b@example.org"]);

        let raw: Vec<u8> = sqlx::query_scalar("SELECT password_enc FROM mail_settings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("pw-geheim-1"));
        // The ciphertext is bound to its column: it cannot be read back as another secret.
        assert!(secrets.decrypt(&raw, "snmp.community").is_err());
    }

    #[tokio::test]
    async fn an_empty_password_keeps_the_stored_one_until_the_user_is_removed() {
        let (_dir, pool) = db().await;
        let secrets = SecretBox::from_key([3u8; 32]);
        save(&pool, &secrets, &input("netmon", Some("pw-geheim-1")))
            .await
            .unwrap();
        save(&pool, &secrets, &input("netmon2", None))
            .await
            .unwrap();
        let config = load_config(&pool, &secrets).await.unwrap();
        assert_eq!(config.username.as_deref(), Some("netmon2"));
        assert_eq!(
            config.password.as_ref().map(Secret::expose),
            Some("pw-geheim-1")
        );

        save(&pool, &secrets, &input("", None)).await.unwrap();
        let view = get_view(&pool).await.unwrap();
        assert!(view.username.is_empty() && !view.password_set);
    }

    #[tokio::test]
    async fn the_notification_log_lists_newest_first_and_clips_text() {
        let (_dir, pool) = db().await;
        let to = vec!["a@example.org".to_owned()];
        record_notification(&pool, 1_000, Kind::Alert, "erste", &to, None)
            .await
            .unwrap();
        record_notification(
            &pool,
            2_000,
            Kind::Test,
            &"x\n".repeat(400),
            &to,
            Some("kaputt"),
        )
        .await
        .unwrap();
        let log = recent_notifications(&pool, 10).await.unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(
            (log[0].kind, log[0].ok, log[0].error.as_str()),
            (Kind::Test, false, "kaputt")
        );
        assert!(
            log[0].subject.chars().count() <= MAX_LOGGED_TEXT && !log[0].subject.contains('\n')
        );
        assert_eq!((log[1].subject.as_str(), log[1].ok), ("erste", true));
        assert_eq!(recent_notifications(&pool, 1).await.unwrap().len(), 1);
    }
}
