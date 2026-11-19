//! E-mail alerts for status changes.
//!
//! The poller hands every status change to a queue; a separate task collects changes that arrive
//! close together (a device going down trips all of its sensors) into one mail and sends it with
//! the settings stored at that moment, so edits in the UI apply immediately. The poller never waits
//! for mail delivery. [`details`] reads what the mail tells beyond the change, [`compose`] writes
//! it (plain text and HTML).

pub mod compose;
pub mod details;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sqlx::SqlitePool;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::crypto::SecretBox;
use crate::error::Result;
use crate::mail;
use crate::mail::view::{Clock, Links};
use crate::status::Status;
use crate::storage::mail::{Kind, load_config, record_notification};
use crate::storage::status::StatusChange;

pub use compose::{Context, Details, Item};

/// How long the first change of a burst waits for companions before the mail goes out.
const BATCH_WINDOW: Duration = Duration::from_secs(10);
const QUEUE_SIZE: usize = 1_000;

/// A status change with the names a human wants to read.
#[derive(Debug, Clone, PartialEq)]
pub struct AlertEvent {
    pub change: StatusChange,
    pub device_id: i64,
    pub device: String,
    pub sensor: String,
    /// For an outage: how many sensors on how many devices it pauses through dependencies (set by
    /// [`deliver`]).
    pub paused_dependents: Option<(usize, usize)>,
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
    let now = now_ms();
    // Sensors paused through a dependency since their change are no news of their own: the outage
    // of the sensor they depend on is, and it says how much it paused.
    let pauses = crate::storage::pauses::load_all(pool, now).await?;
    let roots = pauses.dependency_roots();
    let events: Vec<AlertEvent> = events
        .into_iter()
        .filter(|e| is_alertworthy(&e.change, config.notify_warnings))
        .filter(|e| {
            pauses
                .sensor(e.change.sensor_id, e.device_id)
                .and_then(|w| w.dependency())
                .is_none()
        })
        .map(|mut e| {
            if e.change.to == Status::Down {
                e.paused_dependents = roots
                    .get(&e.change.sensor_id)
                    .map(|a| (a.sensors, a.devices.len()));
            }
            e
        })
        .collect();
    if events.is_empty() {
        return Ok(Delivery::Filtered);
    }
    let lang = config.language;
    let details = details::gather(pool, lang, &pauses, &events, now).await;
    let items: Vec<Item> = events
        .into_iter()
        .zip(details)
        .map(|(event, details)| Item { event, details })
        .collect();
    let context = Context {
        lang,
        links: Links::new(config.ui_url.as_deref()),
        recipients: config.recipients.len(),
        clock: Clock::local(lang, now),
        batch_secs: BATCH_WINDOW.as_secs(),
    };
    let message = compose::compose(&context, &items);
    let outcome = mail::send(&config, &message).await;
    let error = outcome.as_ref().err().map(mail::MailError::user_message);
    let subject = &message.subject;
    record_notification(
        pool,
        now_ms(),
        Kind::Alert,
        subject,
        &config.recipients,
        error.as_ref(),
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
            message: crate::i18n::Msg::Text("Meldung".to_owned()),
            ts_ms: 1_000,
        }
    }

    fn event(device: &str, sensor: &str, from: Option<Status>, to: Status) -> AlertEvent {
        AlertEvent {
            change: change(from, to),
            device_id: 1,
            device: device.to_owned(),
            sensor: sensor.to_owned(),
            paused_dependents: None,
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

    /// A migrated database whose mail settings point at the test server.
    async fn setup(
        smtp: &TestSmtp,
        enabled: bool,
        notify_warnings: bool,
        language: Lang,
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
            language,
            ui_url: String::new(),
        };
        save(&pool, &secrets, &input.check().unwrap())
            .await
            .unwrap();
        (dir, pool, secrets)
    }

    use crate::i18n::Lang;

    #[tokio::test]
    async fn deliver_sends_and_logs() {
        let smtp = TestSmtp::spawn(Behavior::default()).await;
        let (_dir, pool, secrets) = setup(&smtp, true, true, Lang::De).await;
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
        assert_eq!(
            mails[0].subject(),
            "[opennetmon] Ausfall: Switch · Ping – Meldung",
            "only the worthy change"
        );
        assert!(
            mails[0]
                .data
                .contains("Content-Type: multipart/alternative")
        );
        let text = mails[0].part("text/plain").unwrap();
        assert!(text.starts_with("AUSFALL  Switch · Ping\n"), "{text}");
        assert!(!text.contains("Switch · X"));
        let log = recent_notifications(&pool, 5).await.unwrap();
        assert_eq!(
            (
                log.len(),
                log[0].ok,
                log[0].recipients.as_str(),
                log[0].subject.as_str()
            ),
            (
                1,
                true,
                "admin@example.org",
                "[opennetmon] Ausfall: Switch · Ping – Meldung"
            )
        );
    }

    #[tokio::test]
    async fn the_mail_language_comes_from_the_settings() {
        let smtp = TestSmtp::spawn(Behavior::default()).await;
        let (_dir, pool, secrets) = setup(&smtp, true, true, Lang::En).await;
        let events = vec![event("Switch", "Ping", Some(Status::Up), Status::Down)];
        assert_eq!(
            deliver(&pool, &secrets, events).await.unwrap(),
            Delivery::Sent
        );
        let mail = &smtp.received()[0];
        assert_eq!(mail.subject(), "[opennetmon] Down: Switch · Ping – Meldung");
        let html = mail.part("text/html").unwrap();
        assert!(html.contains("<html lang=\"en\">"), "{html}");
        assert!(mail.part("text/plain").unwrap().starts_with("DOWN  "));
    }

    #[tokio::test]
    async fn disabled_filtered_and_failing_deliveries() {
        let smtp = TestSmtp::spawn(Behavior::default()).await;
        let down = || vec![event("S", "P", Some(Status::Up), Status::Down)];
        let (_d1, pool, secrets) = setup(&smtp, false, true, Lang::De).await;
        assert_eq!(
            deliver(&pool, &secrets, down()).await.unwrap(),
            Delivery::Disabled
        );

        let (_d2, pool, secrets) = setup(&smtp, true, false, Lang::De).await;
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
        let (_d3, pool, secrets) = setup(&broken, true, true, Lang::De).await;
        assert_eq!(
            deliver(&pool, &secrets, down()).await.unwrap(),
            Delivery::Failed
        );
        let log = recent_notifications(&pool, 5).await.unwrap();
        assert!(
            !log[0].ok
                && log[0]
                    .error
                    .to_string()
                    .starts_with("Versand fehlgeschlagen"),
            "{log:?}"
        );
    }

    #[tokio::test]
    async fn a_burst_becomes_one_mail_and_shutdown_flushes_the_rest() {
        let smtp = TestSmtp::spawn(Behavior::default()).await;
        let (_dir, pool, secrets) = setup(&smtp, true, true, Lang::De).await;
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
        assert_eq!(
            smtp.received()[0].subject(),
            "[opennetmon] 3 Statuswechsel: 3 Ausfälle"
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
        assert!(
            smtp.received()[1]
                .subject()
                .starts_with("[opennetmon] Ausfall: AP · Ping")
        );
    }

    /// The details come from the database: group path, address, value, limits, availability,
    /// history, and what the outage pauses.
    #[tokio::test]
    async fn the_details_of_an_outage_are_read_from_the_database() {
        use crate::devices::{DeviceInput, Polling};
        use crate::pause::{Dependency, Setting, Target};
        use crate::sensors::spec::SensorSpec;
        use crate::status::{Assessment, Thresholds};
        use crate::storage::sensors::{Sample, create_sensor, get_sensor, set_thresholds};
        use crate::storage::status::record_poll;

        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("d.db")).await.unwrap();
        migrate(&pool).await.unwrap();
        let house = crate::storage::groups::create_group(&pool, "Haus", None)
            .await
            .unwrap();
        let net = crate::storage::groups::create_group(&pool, "Netzwerk", Some(house))
            .await
            .unwrap();
        let input = |name: &str, host: &str| DeviceInput {
            kind: Default::default(),
            name: name.to_owned(),
            host: host.to_owned(),
            group_id: Some(net),
            profile_id: None,
            polling: Polling::default(),
        };
        let switch =
            crate::storage::devices::save_device(&pool, None, input("Switch", "192.0.2.10"))
                .await
                .unwrap();
        let ap =
            crate::storage::devices::save_device(&pool, None, input("AP <Flur>", "192.0.2.21"))
                .await
                .unwrap();
        let ping = create_sensor(&pool, switch, "Ping", &SensorSpec::ping())
            .await
            .unwrap();
        let ap_ping = create_sensor(&pool, ap, "Ping", &SensorSpec::ping())
            .await
            .unwrap();
        let channels = get_sensor(&pool, ping).await.unwrap().unwrap().channels;
        let rtt = channels.iter().find(|c| c.key == "rtt").unwrap();
        let limits = Thresholds {
            warn_above: Some(20.0),
            ..Thresholds::default()
        };
        set_thresholds(&pool, ping, &[(rtt.id, limits)])
            .await
            .unwrap();
        crate::storage::pauses::set_dependency(
            &pool,
            Target::Device(ap),
            Setting {
                dependency: Dependency::Sensor(ping),
                delay_secs: None,
            },
            None,
            now_ms(),
        )
        .await
        .unwrap();

        let now = now_ms();
        let minute = 60_000;
        let ok = Assessment::Healthy("OK".into());
        let fail = Assessment::Failed("keine Antwort".into());
        for k in (2..=60).rev() {
            let ts = now - k * minute;
            let sample = Sample {
                channel_id: rtt.id,
                ts_ms: ts,
                value: 1.5,
            };
            record_poll(&pool, ping, ts, &[sample], &ok).await.unwrap();
            record_poll(&pool, ap_ping, ts, &[], &ok).await.unwrap();
        }
        record_poll(&pool, ping, now - minute, &[], &fail)
            .await
            .unwrap();
        let down = record_poll(&pool, ping, now, &[], &fail)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(down.to, Status::Down);

        let pauses = crate::storage::pauses::load_all(&pool, now).await.unwrap();
        let event = AlertEvent {
            change: down,
            device_id: switch,
            device: "Switch".to_owned(),
            sensor: "Ping".to_owned(),
            paused_dependents: Some((1, 1)),
        };
        // The mail goes out a little after the change (the batch window).
        let sent = now + 10_000;
        let details = details::gather(&pool, Lang::De, &pauses, &[event], sent).await;
        let d = &details[0];
        assert_eq!(d.group_path, ["Haus", "Netzwerk"]);
        assert_eq!(d.host, "192.0.2.10");
        assert_eq!(d.previous_ms, Some(now - minute), "the warning before");
        assert_eq!(
            d.value.as_ref().map(|(v, ts)| (v.as_str(), *ts)),
            Some(("1,5 ms", now - 2 * minute))
        );
        assert_eq!(d.limits, "Warnung über 20 ms");
        assert!(d.availability.is_some(), "{d:?}");
        assert_eq!(d.outages.as_ref().map(|(n, _)| *n), Some(1));
        let history = d.history.as_ref().unwrap();
        assert_eq!(history.bars.len(), 72);
        assert_eq!(history.title, "Antwortzeit, letzte 6 Stunden");
        assert!(
            history.bars.iter().any(|b| b.tint.is_some()),
            "the outage is tinted"
        );
        assert_eq!(
            d.dependents,
            [compose::Dependent {
                name: "AP <Flur>".to_owned(),
                host: "192.0.2.21".to_owned(),
                sensors: 1
            }]
        );
        assert!(
            d.answers && d.others.is_empty(),
            "no other sensors for an outage"
        );
        let png = history.png.as_deref().expect("a picture from the samples");
        assert!(png.starts_with(b"\x89PNG"));
        assert_eq!(history.channel, "Antwortzeit");

        // A sensor without any values keeps the bars and gets no picture.
        let quiet = AlertEvent {
            change: StatusChange {
                sensor_id: ap_ping,
                from: Some(Status::Up),
                to: Status::Down,
                message: crate::i18n::Msg::Text("x".to_owned()),
                ts_ms: now,
            },
            device_id: ap,
            device: "AP".to_owned(),
            sensor: "Ping".to_owned(),
            paused_dependents: None,
        };
        let details = details::gather(&pool, Lang::De, &pauses, &[quiet], sent).await;
        let quiet = details[0].history.as_ref().unwrap();
        assert!(quiet.png.is_none() && quiet.bars.len() == 72);

        // 20 minutes later it answers again: the history shows the outage that just ended.
        let back = now + 20 * minute;
        let sample = |ts_ms, value| Sample {
            channel_id: rtt.id,
            ts_ms,
            value,
        };
        let up = record_poll(&pool, ping, back, &[sample(back, 1.7)], &ok)
            .await
            .unwrap()
            .unwrap();
        let pauses = crate::storage::pauses::load_all(&pool, back).await.unwrap();
        let event = |change| AlertEvent {
            change,
            device_id: switch,
            device: "Switch".to_owned(),
            sensor: "Ping".to_owned(),
            paused_dependents: None,
        };
        let details = details::gather(&pool, Lang::De, &pauses, &[event(up)], back + 10_000).await;
        let d = &details[0];
        assert_eq!(d.previous_ms, Some(now), "the outage began then");
        let tinted = d
            .history
            .as_ref()
            .unwrap()
            .bars
            .iter()
            .filter(|b| b.tint == Some(crate::mail::view::Tone::Down))
            .count();
        assert!(
            tinted >= 4,
            "20 minutes of outage in 5-minute buckets: {tinted}"
        );
        assert!(d.history.as_ref().unwrap().png.is_some());
        assert!(d.average.is_some());

        // Then a warning: the value is beyond the limit; the peak is the one before the change.
        create_sensor(&pool, switch, "Uptime", &SensorSpec::Uptime)
            .await
            .unwrap();
        let hot = back + 10 * minute;
        let warned = record_poll(
            &pool,
            ping,
            hot,
            &[sample(hot, 25.0)],
            &Assessment::Degraded("25 ms".into()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(warned.to, Status::Warning);
        let details =
            details::gather(&pool, Lang::De, &pauses, &[event(warned)], hot + 10_000).await;
        let d = &details[0];
        assert_eq!(d.value.as_ref().map(|(v, _)| v.as_str()), Some("25 ms"));
        assert_eq!(d.beyond.as_deref(), Some("über 20 ms"));
        assert_eq!(
            d.peak
                .as_ref()
                .map(|(label, v, ts)| (*label, v.as_str(), *ts)),
            Some(("mail-fact-peak", "1,7 ms", back))
        );
        assert_eq!(d.trend, None, "no values before the last 2 hours");
        assert_eq!(
            d.others,
            [compose::Other {
                name: "Uptime".to_owned(),
                value: "–".to_owned(),
                status: Status::Unknown,
            }]
        );
    }
}
