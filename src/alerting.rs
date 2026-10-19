//! E-mail alerts for status changes.
//!
//! The poller hands every status change to a queue; a separate task collects changes that arrive
//! close together (a device going down trips all of its sensors) into one mail and sends it with
//! the settings stored at that moment, so edits in the UI apply immediately. The poller never waits
//! for mail delivery.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sqlx::SqlitePool;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::crypto::SecretBox;
use crate::error::Result;
use crate::mail;
use crate::status::Status;
use crate::storage::mail::{Kind, load_config, record_notification};
use crate::storage::status::StatusChange;
use crate::web::format_utc;

/// How long the first change of a burst waits for companions before the mail goes out.
const BATCH_WINDOW: Duration = Duration::from_secs(10);
const QUEUE_SIZE: usize = 1_000;

/// A status change with the names a human wants to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertEvent {
    pub change: StatusChange,
    pub device: String,
    pub sensor: String,
}

/// Whether a change is worth a mail. The first reading that is fine (`None → Up`) is not;
/// warnings can be switched off, in which case their recovery is not mailed either.
pub fn is_alertworthy(change: &StatusChange, notify_warnings: bool) -> bool {
    match (change.from, change.to) {
        (_, Status::Down) => true,
        (_, Status::Warning) => notify_warnings,
        (Some(Status::Down), Status::Up) => true,
        (Some(Status::Warning), Status::Up) => notify_warnings,
        _ => false,
    }
}

fn word(change: &StatusChange) -> &'static str {
    match change.to {
        Status::Down => "Ausfall",
        Status::Warning => "Warnung",
        Status::Up => "Wieder in Ordnung",
        Status::Unknown => "Unbekannt",
        Status::Paused => "Inaktiv",
    }
}

/// Subject and plain-text body for one or more changes.
pub fn compose(events: &[AlertEvent]) -> (String, String) {
    let subject = match events {
        [one] => format!(
            "[opennetmon] {}: {} / {}",
            word(&one.change),
            one.device,
            one.sensor
        ),
        many => {
            let count = |status| many.iter().filter(|e| e.change.to == status).count();
            let parts: Vec<String> = [
                (Status::Down, "Ausfall"),
                (Status::Warning, "Warnung"),
                (Status::Up, "wieder in Ordnung"),
            ]
            .into_iter()
            .filter(|(s, _)| count(*s) > 0)
            .map(|(s, label)| format!("{} {label}", count(s)))
            .collect();
            format!(
                "[opennetmon] {} Statuswechsel: {}",
                many.len(),
                parts.join(", ")
            )
        }
    };
    let mut body = String::new();
    for e in events {
        let from = e.change.from.map_or("erste Abfrage", Status::label);
        body.push_str(&format!(
            "{}  {} / {}\n  {from} -> {}\n  {}\n\n",
            format_utc(e.change.ts_ms),
            e.device,
            e.sensor,
            e.change.to.label(),
            e.change.message,
        ));
    }
    body.push_str("-- \nopennetmon\n");
    (subject, body)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// What became of a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// Notifications are switched off.
    Disabled,
    /// Nothing in the batch was worth a mail.
    Filtered,
    Sent,
    /// Sending failed (the reason is in the notification log).
    Failed,
}

/// Mails a batch with the current settings and logs the outcome.
pub async fn deliver(
    pool: &SqlitePool,
    secrets: &SecretBox,
    events: Vec<AlertEvent>,
) -> Result<Delivery> {
    let config = load_config(pool, secrets).await?;
    if !config.enabled {
        return Ok(Delivery::Disabled);
    }
    let events: Vec<AlertEvent> = events
        .into_iter()
        .filter(|e| is_alertworthy(&e.change, config.notify_warnings))
        .collect();
    if events.is_empty() {
        return Ok(Delivery::Filtered);
    }
    let (subject, body) = compose(&events);
    let outcome = mail::send(&config, &subject, &body).await;
    let error = outcome.as_ref().err().map(mail::MailError::user_message);
    record_notification(
        pool,
        now_ms(),
        Kind::Alert,
        &subject,
        &config.recipients,
        error.as_deref(),
    )
    .await?;
    match error {
        None => {
            tracing::info!(subject = %subject, "alert mail sent");
            Ok(Delivery::Sent)
        }
        Some(message) => {
            tracing::warn!(subject = %subject, error = %message, "alert mail failed");
            Ok(Delivery::Failed)
        }
    }
}

/// Handle of the alert task; the sender side goes to the poller.
pub struct Notifier {
    pub sender: mpsc::Sender<AlertEvent>,
    handle: JoinHandle<()>,
}

impl Notifier {
    pub fn spawn(
        pool: SqlitePool,
        secrets: Arc<SecretBox>,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        Self::spawn_with_window(pool, secrets, shutdown, BATCH_WINDOW)
    }

    pub fn spawn_with_window(
        pool: SqlitePool,
        secrets: Arc<SecretBox>,
        shutdown: watch::Receiver<bool>,
        window: Duration,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(QUEUE_SIZE);
        let handle = tokio::spawn(run(pool, secrets, receiver, shutdown, window));
        Self { sender, handle }
    }

    /// Waits for the task (it flushes a pending batch on shutdown), at most `limit`.
    pub async fn finish(self, limit: Duration) {
        drop(self.sender);
        let abort = self.handle.abort_handle();
        if tokio::time::timeout(limit, self.handle).await.is_err() {
            abort.abort();
        }
    }
}

async fn run(
    pool: SqlitePool,
    secrets: Arc<SecretBox>,
    mut receiver: mpsc::Receiver<AlertEvent>,
    mut shutdown: watch::Receiver<bool>,
    window: Duration,
) {
    loop {
        let first = tokio::select! {
            event = receiver.recv() => match event {
                Some(event) => event,
                None => return,
            },
            _ = shutdown.changed() => return,
        };
        let mut batch = vec![first];
        let deadline = tokio::time::Instant::now() + window;
        let mut stop = false;
        loop {
            tokio::select! {
                event = receiver.recv() => match event {
                    Some(event) => batch.push(event),
                    None => { stop = true; break; }
                },
                _ = tokio::time::sleep_until(deadline) => break,
                _ = shutdown.changed() => { stop = true; break; }
            }
        }
        if stop {
            while let Ok(event) = receiver.try_recv() {
                batch.push(event);
            }
        }
        if let Err(e) = deliver(&pool, &secrets, batch).await {
            tracing::error!(error = %e, "delivering alerts failed");
        }
        if stop {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::Secret;
    use crate::mail::testsmtp::{Behavior, TestSmtp};
    use crate::mail::{MailInput, Security};
    use crate::storage::mail::{recent_notifications, save};
    use crate::storage::{connect, migrate};

    fn change(from: Option<Status>, to: Status) -> StatusChange {
        StatusChange {
            sensor_id: 1,
            from,
            to,
            message: "Meldung".to_owned(),
            ts_ms: 1_000,
        }
    }

    fn event(device: &str, sensor: &str, from: Option<Status>, to: Status) -> AlertEvent {
        AlertEvent {
            change: change(from, to),
            device: device.to_owned(),
            sensor: sensor.to_owned(),
        }
    }

    #[test]
    fn which_changes_are_worth_a_mail() {
        use Status::*;
        let worthy = |from, to, warn| is_alertworthy(&change(from, to), warn);
        assert!(
            !worthy(None, Up, true),
            "a healthy first reading is no news"
        );
        assert!(
            worthy(None, Down, false),
            "a sensor that is down from the start is"
        );
        assert!(worthy(Some(Up), Down, false));
        assert!(worthy(Some(Down), Up, false));
        assert!(worthy(Some(Warning), Down, false));
        assert!(worthy(Some(Up), Warning, true) && worthy(Some(Warning), Up, true));
        assert!(!worthy(Some(Up), Warning, false) && !worthy(Some(Warning), Up, false));
        assert!(!worthy(Some(Up), Unknown, true) && !worthy(Some(Up), Paused, true));
    }

    #[test]
    fn a_single_change_gets_a_specific_subject() {
        let (subject, body) =
            compose(&[event("Switch Büro", "Ping", Some(Status::Up), Status::Down)]);
        assert_eq!(subject, "[opennetmon] Ausfall: Switch Büro / Ping");
        assert!(body.contains("Switch Büro / Ping") && body.contains("In Ordnung -> Ausfall"));
        assert!(body.contains("Meldung") && body.contains("1970-01-01 00:00:01 UTC"));
        let (subject, _) = compose(&[event("AP", "Uptime", Some(Status::Down), Status::Up)]);
        assert_eq!(subject, "[opennetmon] Wieder in Ordnung: AP / Uptime");
        let (_, body) = compose(&[event("AP", "Uptime", None, Status::Down)]);
        assert!(body.contains("erste Abfrage -> Ausfall"));
    }

    #[test]
    fn several_changes_are_summarized() {
        let events = [
            event("Switch", "Ping", Some(Status::Up), Status::Down),
            event("Switch", "Uptime", Some(Status::Up), Status::Down),
            event("Switch", "Port 1", Some(Status::Up), Status::Warning),
        ];
        let (subject, body) = compose(&events);
        assert_eq!(
            subject,
            "[opennetmon] 3 Statuswechsel: 2 Ausfall, 1 Warnung"
        );
        assert_eq!(body.matches("Switch / ").count(), 3);
    }

    /// A migrated database whose mail settings point at the test server.
    async fn setup(
        smtp: &TestSmtp,
        enabled: bool,
        notify_warnings: bool,
    ) -> (tempfile::TempDir, SqlitePool, Arc<SecretBox>) {
        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("a.db")).await.unwrap();
        migrate(&pool).await.unwrap();
        let secrets = Arc::new(SecretBox::from_key([4u8; 32]));
        let input = MailInput {
            enabled,
            host: "127.0.0.1".to_owned(),
            port: smtp.port,
            security: Security::None,
            username: String::new(),
            password: None::<Secret>,
            from_address: "opennetmon@example.org".to_owned(),
            recipients: "admin@example.org".to_owned(),
            notify_warnings,
        };
        save(&pool, &secrets, &input.check().unwrap())
            .await
            .unwrap();
        (dir, pool, secrets)
    }

    #[tokio::test]
    async fn deliver_sends_and_logs() {
        let smtp = TestSmtp::spawn(Behavior::default()).await;
        let (_dir, pool, secrets) = setup(&smtp, true, true).await;
        let events = vec![
            event("Switch", "Ping", Some(Status::Up), Status::Down),
            event("Switch", "X", None, Status::Up),
        ];
        assert_eq!(
            deliver(&pool, &secrets, events).await.unwrap(),
            Delivery::Sent
        );
        let mails = smtp.received();
        assert_eq!(mails.len(), 1);
        assert!(
            mails[0]
                .data
                .contains("Subject: [opennetmon] Ausfall: Switch / Ping"),
            "only the worthy change: {}",
            mails[0].data
        );
        assert!(!mails[0].data.contains("Switch / X"));
        let log = recent_notifications(&pool, 5).await.unwrap();
        assert_eq!(
            (log.len(), log[0].ok, log[0].recipients.as_str()),
            (1, true, "admin@example.org")
        );
    }

    #[tokio::test]
    async fn disabled_filtered_and_failing_deliveries() {
        let smtp = TestSmtp::spawn(Behavior::default()).await;
        let down = || vec![event("S", "P", Some(Status::Up), Status::Down)];
        let (_d1, pool, secrets) = setup(&smtp, false, true).await;
        assert_eq!(
            deliver(&pool, &secrets, down()).await.unwrap(),
            Delivery::Disabled
        );

        let (_d2, pool, secrets) = setup(&smtp, true, false).await;
        let warn = vec![event("S", "P", Some(Status::Up), Status::Warning)];
        assert_eq!(
            deliver(&pool, &secrets, warn).await.unwrap(),
            Delivery::Filtered
        );
        assert!(
            smtp.received().is_empty() && recent_notifications(&pool, 5).await.unwrap().is_empty()
        );

        let broken = TestSmtp::spawn(Behavior {
            reject_recipients: true,
            ..Behavior::default()
        })
        .await;
        let (_d3, pool, secrets) = setup(&broken, true, true).await;
        assert_eq!(
            deliver(&pool, &secrets, down()).await.unwrap(),
            Delivery::Failed
        );
        let log = recent_notifications(&pool, 5).await.unwrap();
        assert!(
            !log[0].ok && log[0].error.starts_with("Versand fehlgeschlagen"),
            "{log:?}"
        );
    }

    #[tokio::test]
    async fn a_burst_becomes_one_mail_and_shutdown_flushes_the_rest() {
        let smtp = TestSmtp::spawn(Behavior::default()).await;
        let (_dir, pool, secrets) = setup(&smtp, true, true).await;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let notifier = Notifier::spawn_with_window(
            pool.clone(),
            secrets,
            shutdown_rx,
            Duration::from_millis(200),
        );
        for sensor in ["Ping", "Uptime", "Port 1"] {
            notifier
                .sender
                .send(event("Switch", sensor, Some(Status::Up), Status::Down))
                .await
                .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(smtp.received().len(), 1, "one mail for the whole burst");
        assert!(
            smtp.received()[0]
                .data
                .contains("3 Statuswechsel: 3 Ausfall")
        );

        // A change still waiting in its window is sent when the server shuts down.
        notifier
            .sender
            .send(event("AP", "Ping", Some(Status::Up), Status::Down))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        shutdown_tx.send(true).unwrap();
        notifier.finish(Duration::from_secs(5)).await;
        assert_eq!(smtp.received().len(), 2);
        assert!(smtp.received()[1].data.contains("Ausfall: AP / Ping"));
    }
}
