//! The poller: one task per device polls its sensors at the device's interval.
//!
//! A supervisor discovers devices (and notices new ones); each device task reloads its device row,
//! sensors and credentials from the database on every cycle, so edits take effect without a
//! restart and a deleted device simply ends its task. Counter baselines live in the task and
//! survive reloads as long as a sensor's configuration is unchanged.
//!
//! - Start times are spread deterministically over the interval (no burst at a full minute).
//! - A global [`Semaphore`] caps how many devices are polled at the same time.
//! - Sensors of one device run one after another; once a device-level SNMP error occurs
//!   (timeout, authentication, unreachable) the remaining SNMP sensors of that cycle are not tried.
//! - SNMP sessions are opened per cycle: it costs one round trip and avoids stale SNMPv3 engine state.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sqlx::SqlitePool;
use tokio::sync::{Semaphore, mpsc, watch};
use tokio::task::JoinHandle;

use crate::alerting::{AlertEvent, Notifier};
use crate::crypto::SecretBox;
use crate::devices::Device;
use crate::error::Result;
use crate::maintenance::{self, Retention};
use crate::sensors::ping::PingClients;
use crate::sensors::spec::SensorSpec;
use crate::sensors::{PollContext, Sensor, SensorError};
use crate::snmp::{SnmpClient, SnmpError, SnmpTarget};
use crate::status::{Assessment, ChannelRules, assess_with};
use crate::storage::devices::get_device;
use crate::storage::sensors::{Sample, SensorRecord, list_sensors};
use crate::storage::status::{StatusChange, record_poll};
use crate::storage::{credentials, devices};

/// How often the supervisor looks for new devices.
/// Message for SNMP sensors of a ping-only device.
const NO_SNMP: &str = "Für dieses Gerät ist kein SNMP eingerichtet (nur Ping).";
const RESCAN_INTERVAL: Duration = Duration::from_secs(15);
/// The first poll of a device is delayed by at most this much, however long its interval is.
const MAX_START_OFFSET: Duration = Duration::from_secs(30);
/// Extra time on top of the device's own timeouts before a sensor poll is cut off.
const POLL_GRACE: Duration = Duration::from_secs(2);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);
const NOTIFIER_GRACE: Duration = Duration::from_secs(20);
const MAINTENANCE_FIRST_RUN: Duration = Duration::from_secs(30);
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(600);

pub struct Poller {
    shutdown: watch::Sender<bool>,
    supervisor: JoinHandle<()>,
    maintenance: JoinHandle<()>,
    notifier: Notifier,
}

/// A status change of one sensor of the polled device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SensorChange {
    pub change: StatusChange,
    pub sensor: String,
}

struct Shared {
    alerts: mpsc::Sender<AlertEvent>,
    pool: SqlitePool,
    secrets: Arc<SecretBox>,
    ping: PingClients,
    permits: Semaphore,
}

impl Poller {
    /// Starts polling and the periodic rollup/retention run. `concurrency` is the number of
    /// devices polled in parallel.
    pub fn spawn(
        pool: SqlitePool,
        secrets: Arc<SecretBox>,
        concurrency: usize,
        retention: Retention,
    ) -> Self {
        let (shutdown, signal) = watch::channel(false);
        let maintenance = tokio::spawn(maintain(pool.clone(), retention, signal.clone()));
        let notifier = Notifier::spawn(pool.clone(), Arc::clone(&secrets), signal.clone());
        let ping = PingClients::new();
        if !ping.ipv4_available() {
            tracing::warn!(
                "cannot open ICMP sockets: ping sensors will fail; allow them with \
                 `sysctl net.ipv4.ping_group_range=\"0 2147483647\"` or run the service with CAP_NET_RAW"
            );
        }
        let shared = Arc::new(Shared {
            alerts: notifier.sender.clone(),
            pool,
            secrets,
            ping,
            permits: Semaphore::new(concurrency.max(1)),
        });
        let supervisor = tokio::spawn(supervise(shared, signal));
        Self {
            shutdown,
            supervisor,
            maintenance,
            notifier,
        }
    }

    /// Stops all device tasks and waits for them.
    pub async fn stop(self) {
        let _ = self.shutdown.send(true);
        let _ = self.supervisor.await;
        let _ = self.maintenance.await;
        // Pending alerts are still mailed on the way out.
        self.notifier.finish(NOTIFIER_GRACE).await;
    }
}

/// Rolls measurements up and applies the retention: shortly after start, then periodically.
async fn maintain(pool: SqlitePool, retention: Retention, mut shutdown: watch::Receiver<bool>) {
    let mut pause = MAINTENANCE_FIRST_RUN;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(pause) => {}
            _ = shutdown.changed() => return,
        }
        pause = MAINTENANCE_INTERVAL;
        match maintenance::run(&pool, now_ms(), retention).await {
            Ok(report) => {
                tracing::debug!(?report, "maintenance finished");
                if report.raw_deleted
                    + report.hourly_deleted
                    + report.events_deleted
                    + report.notifications_deleted
                    > 0
                {
                    tracing::info!(?report, "old data deleted");
                }
            }
            Err(e) => tracing::error!(error = %e, "maintenance failed"),
        }
    }
}

async fn supervise(shared: Arc<Shared>, mut shutdown: watch::Receiver<bool>) {
    let mut tasks: HashMap<i64, JoinHandle<()>> = HashMap::new();
    loop {
        match devices::list_devices(&shared.pool).await {
            Ok(list) => {
                tasks.retain(|_, handle| !handle.is_finished());
                let known: HashSet<i64> = tasks.keys().copied().collect();
                for device in list.iter().filter(|d| !known.contains(&d.id)) {
                    tracing::debug!(device = %device.name, "starting poll task");
                    tasks.insert(
                        device.id,
                        tokio::spawn(device_loop(
                            Arc::clone(&shared),
                            device.id,
                            shutdown.clone(),
                        )),
                    );
                }
            }
            Err(e) => tracing::error!(error = %e, "listing devices failed"),
        }
        tokio::select! {
            _ = tokio::time::sleep(RESCAN_INTERVAL) => {}
            _ = shutdown.changed() => break,
        }
    }
    let deadline = Instant::now() + SHUTDOWN_GRACE;
    for (_, handle) in tasks {
        let left = deadline.saturating_duration_since(Instant::now());
        let abort = handle.abort_handle();
        if tokio::time::timeout(left, handle).await.is_err() {
            abort.abort();
        }
    }
}

/// Deterministic start delay in `[0, min(interval, MAX_START_OFFSET))`, spread by device id.
pub fn start_offset(device_id: i64, interval: Duration) -> Duration {
    let slot = (device_id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 54; // 0..1024
    interval.min(MAX_START_OFFSET) * u32::try_from(slot).unwrap_or(0) / 1024
}

async fn device_loop(shared: Arc<Shared>, device_id: i64, mut shutdown: watch::Receiver<bool>) {
    let mut state = SensorSet::default();
    let mut first = true;
    let mut next = tokio::time::Instant::now();
    loop {
        let device = match get_device(&shared.pool, device_id).await {
            Ok(Some(device)) => device,
            Ok(None) => return, // deleted
            Err(e) => {
                tracing::error!(device_id, error = %e, "loading device failed");
                if cancelled_during(&mut shutdown, Duration::from_secs(10)).await {
                    return;
                }
                continue;
            }
        };
        let interval = Duration::from_secs(u64::from(device.polling.interval_secs));
        if first {
            first = false;
            next += start_offset(device_id, interval);
        }
        tokio::select! {
            _ = tokio::time::sleep_until(next) => {}
            _ = shutdown.changed() => return,
        }

        let permit = tokio::select! {
            permit = shared.permits.acquire() => permit,
            _ = shutdown.changed() => return,
        };
        if permit.is_ok() {
            match poll_device(&shared, device_id, &mut state).await {
                Ok(Some(changes)) => {
                    for change in &changes {
                        log_change(&device, &change.change);
                    }
                    queue_alerts(&shared.alerts, &device.name, changes);
                }
                Ok(None) => return,
                Err(e) => tracing::error!(device = %device.name, error = %e, "poll failed"),
            }
        }
        drop(permit);

        // Fixed schedule; if a cycle overran, continue right away instead of bursting.
        next = (next + interval).max(tokio::time::Instant::now());
    }
}

async fn cancelled_during(shutdown: &mut watch::Receiver<bool>, pause: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(pause) => false,
        _ = shutdown.changed() => true,
    }
}

/// Hands status changes to the alert queue without ever blocking the poller.
fn queue_alerts(alerts: &mpsc::Sender<AlertEvent>, device: &str, changes: Vec<SensorChange>) {
    for SensorChange { change, sensor } in changes {
        let event = AlertEvent {
            change,
            device: device.to_owned(),
            sensor,
        };
        if alerts.try_send(event).is_err() {
            tracing::warn!(device, "alert queue is full or closed; dropping an alert");
        }
    }
}

fn log_change(device: &Device, change: &StatusChange) {
    tracing::info!(
        device = %device.name,
        sensor_id = change.sensor_id,
        from = change.from.map_or("none", |s| s.as_str()),
        to = change.to.as_str(),
        message = %change.message,
        "sensor status changed"
    );
}

/// Sensor instances of one device, keyed by sensor id. Rebuilt only when the configuration changes.
#[derive(Default)]
pub struct SensorSet {
    sensors: HashMap<i64, (SensorSpec, Box<dyn Sensor>)>,
}

impl SensorSet {
    fn reconcile(&mut self, records: &[SensorRecord]) {
        let live: HashSet<i64> = records.iter().map(|r| r.id).collect();
        self.sensors.retain(|id, _| live.contains(id));
        for record in records {
            let stale = self
                .sensors
                .get(&record.id)
                .is_none_or(|(spec, _)| *spec != record.spec);
            if stale {
                self.sensors
                    .insert(record.id, (record.spec.clone(), record.spec.build()));
            }
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Why SNMP is unusable for the whole device in this cycle.
fn device_level(error: &SensorError) -> bool {
    matches!(
        error,
        SensorError::Snmp(SnmpError::Timeout | SnmpError::Auth | SnmpError::Unreachable)
    )
}

/// Polls all enabled sensors of one device once and stores the results.
/// Returns `None` if the device no longer exists, otherwise the status changes of this cycle.
pub async fn poll_device_once(
    pool: &SqlitePool,
    secrets: &SecretBox,
    ping: &PingClients,
    device_id: i64,
    state: &mut SensorSet,
) -> Result<Option<Vec<SensorChange>>> {
    let Some(device) = get_device(pool, device_id).await? else {
        return Ok(None);
    };
    let records: Vec<SensorRecord> = list_sensors(pool, device_id)
        .await?
        .into_iter()
        .filter(|r| r.enabled)
        .collect();
    state.reconcile(&records);
    if records.is_empty() {
        return Ok(Some(Vec::new()));
    }

    let polling = &device.polling;
    let timeout = Duration::from_secs(u64::from(polling.timeout_secs));
    let target = SnmpTarget {
        host: device.host.clone(),
        port: polling.port,
        timeout,
        retries: polling.retries,
    };
    let cutoff = timeout * (polling.retries + 1) + POLL_GRACE;

    // Open the SNMP session only if some sensor needs it; a failure is reported per sensor.
    let mut snmp_failure: Option<String> = None;
    let mut client: Option<SnmpClient> = None;
    if records.iter().any(|r| r.spec.uses_snmp()) {
        let loaded = match device.profile_id {
            Some(profile) => credentials::load_credentials(pool, secrets, profile).await,
            None => Ok(None),
        };
        match loaded {
            Ok(Some(creds)) => match SnmpClient::connect(&target, &creds).await {
                Ok(c) => client = Some(c),
                Err(e) => snmp_failure = Some(e.user_message()),
            },
            Ok(None) if device.profile_id.is_none() => {
                snmp_failure = Some(NO_SNMP.to_owned());
            }
            Ok(None) => {
                snmp_failure = Some("Das SNMP-Profil des Geräts existiert nicht mehr.".to_owned())
            }
            Err(e) => {
                tracing::error!(device = %device.name, error = %e, "cannot load credentials");
                snmp_failure = Some("Die SNMP-Zugangsdaten sind nicht lesbar.".to_owned());
            }
        }
    }

    let mut changes = Vec::new();
    for record in &records {
        let Some((spec, sensor)) = state.sensors.get_mut(&record.id) else {
            continue;
        };
        let started = now_ms();
        let assessment_and_values = if spec.uses_snmp() && snmp_failure.is_some() {
            (
                Assessment::Failed(snmp_failure.clone().unwrap_or_default()),
                Vec::new(),
            )
        } else {
            let mut ctx = PollContext {
                host: &device.host,
                now_ms: started,
                timeout,
                ping,
                snmp: client.as_mut(),
            };
            let result = match tokio::time::timeout(cutoff, sensor.poll(&mut ctx)).await {
                Ok(result) => result,
                Err(_) => Err(SensorError::Snmp(SnmpError::Timeout)),
            };
            if let Err(e) = &result
                && spec.uses_snmp()
                && device_level(e)
            {
                snmp_failure = Some(e.user_message());
            }
            let samples = match &result {
                Ok(m) => m
                    .channels
                    .iter()
                    .filter_map(|reading| {
                        let value = reading.value.filter(|v| v.is_finite())?;
                        let channel = record.channels.iter().find(|c| c.key == reading.key)?;
                        Some(Sample {
                            channel_id: channel.id,
                            ts_ms: started,
                            value,
                        })
                    })
                    .collect(),
                Err(_) => Vec::new(),
            };
            let rules = |key: &str| {
                record
                    .channels
                    .iter()
                    .find(|c| c.key == key)
                    .map(|c| ChannelRules {
                        thresholds: c.thresholds,
                        states: c.states.clone(),
                    })
                    .unwrap_or_default()
            };
            (assess_with(spec, &result, &rules), samples)
        };
        let (assessment, samples) = assessment_and_values;
        if let Some(change) = record_poll(pool, record.id, started, &samples, &assessment).await? {
            changes.push(SensorChange {
                change,
                sensor: record.name.clone(),
            });
        }
    }
    Ok(Some(changes))
}

async fn poll_device(
    shared: &Shared,
    device_id: i64,
    state: &mut SensorSet,
) -> Result<Option<Vec<SensorChange>>> {
    poll_device_once(
        &shared.pool,
        &shared.secrets,
        &shared.ping,
        device_id,
        state,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::{ProfileInput, ProfileKind, Secret};
    use crate::devices::{DeviceInput, Polling};
    use crate::sensors::snmp::OidMode;
    use crate::snmp::testagent::{FakeAgent, FakeValue};
    use crate::status::Status;
    use crate::storage::sensors::{create_sensor, delete_sensor, samples_since, set_enabled};
    use crate::storage::status::{get_state, recent_events};
    use crate::storage::{connect, migrate};

    struct Env {
        _dir: tempfile::TempDir,
        pool: SqlitePool,
        secrets: SecretBox,
        ping: PingClients,
        agent: FakeAgent,
        device: i64,
    }

    async fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("p.db")).await.unwrap();
        migrate(&pool).await.unwrap();
        let secrets = SecretBox::from_key([9u8; 32]);
        let profile = credentials::save_profile(
            &pool,
            &secrets,
            None,
            &ProfileInput {
                name: "fake".to_owned(),
                kind: ProfileKind::V2c {
                    community: Some(Secret::new("public")),
                },
            },
        )
        .await
        .unwrap();
        let agent = FakeAgent::spawn("public", "fake switch", 1_000).await;
        let device = devices::save_device(
            &pool,
            None,
            DeviceInput {
                name: "dev".to_owned(),
                host: "127.0.0.1".to_owned(),
                group_id: None,
                profile_id: Some(profile),
                polling: Polling {
                    port: agent.port,
                    interval_secs: 10,
                    timeout_secs: 1,
                    retries: 0,
                },
            },
        )
        .await
        .unwrap();
        Env {
            _dir: dir,
            pool,
            secrets,
            ping: PingClients::new(),
            agent,
            device,
        }
    }

    impl Env {
        async fn poll(&self, state: &mut SensorSet) -> Vec<StatusChange> {
            poll_device_once(&self.pool, &self.secrets, &self.ping, self.device, state)
                .await
                .unwrap()
                .unwrap()
                .into_iter()
                .map(|c| c.change)
                .collect()
        }

        async fn state_of(&self, sensor: i64) -> Status {
            get_state(&self.pool, sensor).await.unwrap().unwrap().status
        }
    }

    #[tokio::test]
    async fn a_cycle_stores_samples_and_marks_the_sensor_up() {
        let env = env().await;
        let sensor = create_sensor(&env.pool, env.device, "Uptime", &SensorSpec::Uptime)
            .await
            .unwrap();
        env.agent.set_uptime(250_000);
        let mut state = SensorSet::default();
        let changes = env.poll(&mut state).await;
        assert_eq!(changes.len(), 1);
        assert_eq!((changes[0].from, changes[0].to), (None, Status::Up));

        let record = crate::storage::sensors::get_sensor(&env.pool, sensor)
            .await
            .unwrap()
            .unwrap();
        let samples = samples_since(&env.pool, record.channels[0].id, 0)
            .await
            .unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].value, 2500.0);
        assert!(env.poll(&mut state).await.is_empty(), "no change, no event");
    }

    #[tokio::test]
    async fn an_unreachable_device_warns_then_goes_down_and_recovers() {
        let env = env().await;
        let sensors = [
            create_sensor(&env.pool, env.device, "Uptime", &SensorSpec::Uptime)
                .await
                .unwrap(),
            create_sensor(
                &env.pool,
                env.device,
                "Status",
                &SensorSpec::InterfaceStatus { if_index: 1 },
            )
            .await
            .unwrap(),
        ];
        env.agent
            .set(&[1, 3, 6, 1, 2, 1, 2, 2, 1, 8, 1], FakeValue::Integer(1));
        let mut state = SensorSet::default();
        env.poll(&mut state).await;
        assert_eq!(env.state_of(sensors[0]).await, Status::Up);

        // Point the device at a port where nobody answers.
        let dead = FakeAgent::spawn("other", "x", 1).await; // wrong community -> silence
        sqlx::query("UPDATE devices SET port = ? WHERE id = ?")
            .bind(i64::from(dead.port))
            .bind(env.device)
            .execute(&env.pool)
            .await
            .unwrap();
        let started = Instant::now();
        env.poll(&mut state).await;
        assert!(
            started.elapsed() < Duration::from_millis(2_500),
            "the second sensor must not wait for its own timeout"
        );
        for s in sensors {
            assert_eq!(env.state_of(s).await, Status::Warning);
        }
        env.poll(&mut state).await;
        for s in sensors {
            assert_eq!(env.state_of(s).await, Status::Down);
        }

        sqlx::query("UPDATE devices SET port = ? WHERE id = ?")
            .bind(i64::from(env.agent.port))
            .bind(env.device)
            .execute(&env.pool)
            .await
            .unwrap();
        env.poll(&mut state).await;
        for s in sensors {
            assert_eq!(env.state_of(s).await, Status::Up);
        }
        let path: Vec<_> = recent_events(&env.pool, 50)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.sensor_id == sensors[0])
            .map(|e| e.to)
            .collect();
        assert_eq!(
            path,
            [Status::Up, Status::Down, Status::Warning, Status::Up]
        );
    }

    #[tokio::test]
    async fn an_interface_that_is_down_makes_the_sensor_down_at_once() {
        let env = env().await;
        let sensor = create_sensor(
            &env.pool,
            env.device,
            "Port 1",
            &SensorSpec::InterfaceStatus { if_index: 1 },
        )
        .await
        .unwrap();
        let oid = [1, 3, 6, 1, 2, 1, 2, 2, 1, 8, 1];
        env.agent.set(&oid, FakeValue::Integer(2));
        let mut state = SensorSet::default();
        env.poll(&mut state).await;
        assert_eq!(env.state_of(sensor).await, Status::Down);
        env.agent.set(&oid, FakeValue::Integer(1));
        env.poll(&mut state).await;
        assert_eq!(env.state_of(sensor).await, Status::Up);
    }

    #[tokio::test]
    async fn snmp_sensors_of_a_device_without_snmp_report_why() {
        let env = env().await;
        sqlx::query("UPDATE devices SET profile_id = NULL WHERE id = ?")
            .bind(env.device)
            .execute(&env.pool)
            .await
            .unwrap();
        let sensor = create_sensor(&env.pool, env.device, "Uptime", &SensorSpec::Uptime)
            .await
            .unwrap();
        let mut state = SensorSet::default();
        env.poll(&mut state).await;
        let stored = get_state(&env.pool, sensor).await.unwrap().unwrap();
        assert_eq!(
            (stored.status, stored.message.as_str()),
            (Status::Warning, NO_SNMP)
        );
    }

    #[tokio::test]
    async fn rated_status_values_set_the_state_from_the_next_cycle() {
        let env = env().await;
        let oid = vec![1, 3, 6, 1, 4, 1, 11863, 6, 56, 1, 1, 2, 1, 11, 1];
        let sensor = create_sensor(
            &env.pool,
            env.device,
            "PoE-Status Port 1",
            &SensorSpec::Oid {
                oid: oid.clone(),
                mode: OidMode::Gauge,
                scale: 1.0,
                unit: None,
            },
        )
        .await
        .unwrap();
        env.agent.set(&oid, FakeValue::Integer(3));
        let mut state = SensorSet::default();
        env.poll(&mut state).await;
        assert_eq!(
            env.state_of(sensor).await,
            Status::Up,
            "unrated: just a number"
        );

        let channel = crate::storage::sensors::get_sensor(&env.pool, sensor)
            .await
            .unwrap()
            .unwrap()
            .channels[0]
            .id;
        let rate = |value, label: &str, status| crate::status::ValueState {
            value,
            label: label.to_owned(),
            status,
        };
        crate::storage::sensors::set_value_states(
            &env.pool,
            sensor,
            channel,
            &[rate(2, "on", Status::Up), rate(3, "overload", Status::Down)],
        )
        .await
        .unwrap();
        env.poll(&mut state).await;
        let stored = get_state(&env.pool, sensor).await.unwrap().unwrap();
        assert_eq!(
            (stored.status, stored.message.as_str()),
            (Status::Down, "overload (3) gilt als Ausfall")
        );
        env.agent.set(&oid, FakeValue::Integer(2));
        env.poll(&mut state).await;
        let stored = get_state(&env.pool, sensor).await.unwrap().unwrap();
        assert_eq!(
            (stored.status, stored.message.as_str()),
            (Status::Up, "on (2)")
        );
    }

    #[tokio::test]
    async fn counter_baselines_survive_cycles_and_rates_appear_from_the_second() {
        let env = env().await;
        let oid = vec![1, 3, 6, 1, 4, 1, 99, 1, 0];
        let sensor = create_sensor(
            &env.pool,
            env.device,
            "Counter",
            &SensorSpec::Oid {
                oid: oid.clone(),
                mode: OidMode::Counter,
                scale: 1.0,
                unit: None,
            },
        )
        .await
        .unwrap();
        let mut state = SensorSet::default();
        env.agent.set(&oid, FakeValue::Counter32(100));
        env.poll(&mut state).await;
        env.agent.set(&oid, FakeValue::Counter32(100));
        env.poll(&mut state).await;
        let record = crate::storage::sensors::get_sensor(&env.pool, sensor)
            .await
            .unwrap()
            .unwrap();
        let samples = samples_since(&env.pool, record.channels[0].id, 0)
            .await
            .unwrap();
        assert_eq!(samples.len(), 1, "the first poll only sets the baseline");
        assert_eq!(samples[0].value, 0.0);
    }

    #[tokio::test]
    async fn thresholds_move_a_sensor_between_up_warning_and_down() {
        use crate::status::Thresholds;
        use crate::storage::sensors::{get_sensor, set_thresholds};
        let env = env().await;
        let oid = vec![1, 3, 6, 1, 4, 1, 99, 5, 0];
        let sensor = create_sensor(
            &env.pool,
            env.device,
            "Temp",
            &SensorSpec::Oid {
                oid: oid.clone(),
                mode: OidMode::Gauge,
                scale: 1.0,
                unit: None,
            },
        )
        .await
        .unwrap();
        let channel = get_sensor(&env.pool, sensor)
            .await
            .unwrap()
            .unwrap()
            .channels[0]
            .id;
        let limits = Thresholds {
            warn_above: Some(100.0),
            down_above: Some(300.0),
            ..Thresholds::default()
        };
        set_thresholds(&env.pool, sensor, &[(channel, limits)])
            .await
            .unwrap();

        let mut state = SensorSet::default();
        for (value, expected) in [
            (50, Status::Up),
            (150, Status::Warning),
            (400, Status::Down),
            (50, Status::Up),
        ] {
            env.agent.set(&oid, FakeValue::Gauge32(value));
            env.poll(&mut state).await;
            assert_eq!(env.state_of(sensor).await, expected, "value {value}");
        }
        let message = get_state(&env.pool, sensor).await.unwrap().unwrap().message;
        assert_eq!(message, "OK");
        let events = recent_events(&env.pool, 10).await.unwrap();
        assert!(
            events
                .iter()
                .any(|e| e.to == Status::Down && e.message.contains("Ausfallgrenze 300")),
            "{events:?}"
        );
    }

    #[tokio::test]
    async fn a_device_going_down_produces_one_summary_mail() {
        use crate::mail::testsmtp::{Behavior, TestSmtp};
        use crate::mail::{MailInput, Security};
        let env = env().await;
        let smtp = TestSmtp::spawn(Behavior::default()).await;
        let input = MailInput {
            enabled: true,
            host: "127.0.0.1".to_owned(),
            port: smtp.port,
            security: Security::None,
            username: String::new(),
            password: None,
            from_address: "netmon@example.org".to_owned(),
            recipients: "admin@example.org".to_owned(),
            notify_warnings: true,
        };
        crate::storage::mail::save(&env.pool, &env.secrets, &input.check().unwrap())
            .await
            .unwrap();
        for name in ["Uptime", "Ping"] {
            let spec = if name == "Ping" {
                SensorSpec::Ping
            } else {
                SensorSpec::Uptime
            };
            create_sensor(&env.pool, env.device, name, &spec)
                .await
                .unwrap();
        }

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let notifier = Notifier::spawn_with_window(
            env.pool.clone(),
            Arc::new(SecretBox::from_key([9u8; 32])),
            shutdown_rx,
            Duration::from_millis(300),
        );
        let mut state = SensorSet::default();
        let queue = |changes: Vec<SensorChange>| queue_alerts(&notifier.sender, "dev", changes);
        let first = poll_device_once(&env.pool, &env.secrets, &env.ping, env.device, &mut state)
            .await
            .unwrap()
            .unwrap();
        queue(first); // first readings: Up, no mail

        // The device stops answering SNMP: Warning, then Down for the SNMP sensor.
        let dead = FakeAgent::spawn("other", "x", 1).await;
        sqlx::query("UPDATE devices SET port = ? WHERE id = ?")
            .bind(i64::from(dead.port))
            .bind(env.device)
            .execute(&env.pool)
            .await
            .unwrap();
        let mut burst = Vec::new();
        for _ in 0..2 {
            burst.extend(
                poll_device_once(&env.pool, &env.secrets, &env.ping, env.device, &mut state)
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        // Delivered together, as when several sensors of one device fail in the same cycle.
        queue(burst);
        tokio::time::sleep(Duration::from_millis(900)).await;
        shutdown_tx.send(true).unwrap();
        notifier.finish(Duration::from_secs(5)).await;

        let mails = smtp.received();
        assert_eq!(
            mails.len(),
            1,
            "all changes of the burst in one mail: {mails:?}"
        );
        assert!(mails[0].data.contains("dev / Uptime"), "{}", mails[0].data);
        assert!(mails[0].data.contains("Ausfall"), "{}", mails[0].data);
        assert!(
            !mails[0].data.contains("erste Abfrage -> In Ordnung"),
            "first Up readings are not news"
        );
    }

    #[tokio::test]
    async fn disabled_and_deleted_sensors_are_not_polled() {
        let env = env().await;
        let a = create_sensor(&env.pool, env.device, "A", &SensorSpec::Uptime)
            .await
            .unwrap();
        let b = create_sensor(&env.pool, env.device, "B", &SensorSpec::Uptime)
            .await
            .unwrap();
        let mut state = SensorSet::default();
        env.poll(&mut state).await;
        set_enabled(&env.pool, a, false).await.unwrap();
        delete_sensor(&env.pool, b).await.unwrap();
        env.poll(&mut state).await;
        assert!(state.sensors.is_empty());
        assert_eq!(
            get_state(&env.pool, a).await.unwrap().unwrap().status,
            Status::Up
        );
    }

    #[tokio::test]
    async fn a_missing_device_ends_polling() {
        let env = env().await;
        let result = poll_device_once(
            &env.pool,
            &env.secrets,
            &env.ping,
            env.device + 100,
            &mut SensorSet::default(),
        )
        .await
        .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn a_device_without_sensors_is_a_quiet_no_op() {
        let env = env().await;
        assert!(env.poll(&mut SensorSet::default()).await.is_empty());
    }

    #[test]
    fn start_offsets_spread_devices_within_the_interval() {
        let interval = Duration::from_secs(60);
        let offsets: Vec<_> = (1..=7).map(|id| start_offset(id, interval)).collect();
        assert!(offsets.iter().all(|o| *o < MAX_START_OFFSET));
        let distinct: HashSet<_> = offsets.iter().collect();
        assert_eq!(distinct.len(), 7, "{offsets:?}");
        assert!(start_offset(1, Duration::from_secs(10)) < Duration::from_secs(10));
        assert_eq!(start_offset(3, interval), start_offset(3, interval));
    }

    #[tokio::test]
    async fn the_poller_starts_and_stops_promptly() {
        let env = env().await;
        create_sensor(&env.pool, env.device, "Uptime", &SensorSpec::Uptime)
            .await
            .unwrap();
        let poller = Poller::spawn(
            env.pool.clone(),
            Arc::new(SecretBox::from_key([9u8; 32])),
            4,
            Retention {
                raw_days: 7,
                hourly_days: 365,
            },
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        let started = Instant::now();
        poller.stop().await;
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
