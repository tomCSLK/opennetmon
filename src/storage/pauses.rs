//! Persistence of pauses (see [`crate::pause`]): the pause columns of `sensors`, `devices` and
//! `groups` and the log `pause_log`; the dependency settings of the same tables and the log
//! `dependency_log`.

use std::collections::HashSet;

use sqlx::{FromRow, SqliteConnection, SqlitePool};

use crate::error::{Error, Result};
use crate::i18n::Msg;
use crate::pause::{Dependency, Observed, Pause, Pauses, SensorInfo, Setting, Target};
use crate::status::Status;

/// Entries kept in `pause_log` and in `dependency_log`; older ones are deleted whenever a new one
/// is written.
pub const LOG_KEPT: i64 = 500;

/// What happened to a pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Paused,
    Resumed,
    /// A timed pause ran out.
    Expired,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Paused => "paused",
            Self::Resumed => "resumed",
            Self::Expired => "expired",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [Self::Paused, Self::Resumed, Self::Expired]
            .into_iter()
            .find(|a| a.as_str() == s)
    }
}

fn own(paused: bool, until_ms: Option<i64>, reason: String) -> Option<Pause> {
    paused.then_some(Pause { until_ms, reason })
}

#[derive(FromRow)]
struct GroupRow {
    id: i64,
    name: String,
    parent_id: Option<i64>,
    paused: bool,
    pause_until_ms: Option<i64>,
    pause_reason: String,
    dependency: String,
    dependency_sensor_id: Option<i64>,
    dependency_delay_secs: Option<i64>,
}

#[derive(FromRow)]
struct DeviceRow {
    id: i64,
    group_id: Option<i64>,
    paused: bool,
    pause_until_ms: Option<i64>,
    pause_reason: String,
    dependency: String,
    dependency_sensor_id: Option<i64>,
    dependency_delay_secs: Option<i64>,
}

#[derive(FromRow)]
struct SensorRow {
    id: i64,
    device_id: i64,
    name: String,
    device_name: String,
    enabled: bool,
    pause_until_ms: Option<i64>,
    pause_reason: String,
    dependency: String,
    dependency_sensor_id: Option<i64>,
    dependency_delay_secs: Option<i64>,
    dependency_master: bool,
    status: Option<String>,
    consecutive_failures: Option<i64>,
    change_ts: Option<i64>,
    change_from: Option<String>,
}

fn setting(kind: &str, sensor: Option<i64>, delay: Option<i64>) -> Setting {
    Setting {
        dependency: Dependency::from_columns(kind, sensor),
        // Values outside the column's CHECK cannot be stored; read anything else as unset.
        delay_secs: delay.and_then(|d| u32::try_from(d).ok()),
    }
}

/// All pauses as of `now_ms`, with the dependency settings and the last state of every sensor.
pub async fn load(conn: &mut SqliteConnection, now_ms: i64) -> Result<Pauses> {
    let mut pauses = Pauses {
        now_ms,
        ..Pauses::default()
    };
    let groups: Vec<GroupRow> = sqlx::query_as(
        "SELECT id, name, parent_id, paused, pause_until_ms, pause_reason, dependency, \
         dependency_sensor_id, dependency_delay_secs FROM groups",
    )
    .fetch_all(&mut *conn)
    .await?;
    for g in groups {
        pauses.group_settings.insert(
            g.id,
            setting(
                &g.dependency,
                g.dependency_sensor_id,
                g.dependency_delay_secs,
            ),
        );
        pauses.groups.insert(
            g.id,
            (
                g.name,
                g.parent_id,
                own(g.paused, g.pause_until_ms, g.pause_reason),
            ),
        );
    }
    let devices: Vec<DeviceRow> = sqlx::query_as(
        "SELECT id, group_id, paused, pause_until_ms, pause_reason, dependency, \
         dependency_sensor_id, dependency_delay_secs FROM devices",
    )
    .fetch_all(&mut *conn)
    .await?;
    for d in devices {
        pauses.device_settings.insert(
            d.id,
            setting(
                &d.dependency,
                d.dependency_sensor_id,
                d.dependency_delay_secs,
            ),
        );
        pauses.devices.insert(
            d.id,
            (d.group_id, own(d.paused, d.pause_until_ms, d.pause_reason)),
        );
    }
    // The last status change per sensor tells whether it just came back (for the delay).
    let sensors: Vec<SensorRow> = sqlx::query_as(
        "SELECT s.id, s.device_id, s.name, d.name AS device_name, s.enabled, s.pause_until_ms, \
         s.pause_reason, s.dependency, s.dependency_sensor_id, s.dependency_delay_secs, \
         s.dependency_master, st.status, st.consecutive_failures, \
         e.ts_ms AS change_ts, e.from_status AS change_from \
         FROM sensors s JOIN devices d ON d.id = s.device_id \
         LEFT JOIN sensor_status st ON st.sensor_id = s.id \
         LEFT JOIN events e ON e.id = (SELECT MAX(id) FROM events WHERE sensor_id = s.id)",
    )
    .fetch_all(&mut *conn)
    .await?;
    for r in sensors {
        if !r.enabled {
            pauses.sensors.insert(
                r.id,
                Pause {
                    until_ms: r.pause_until_ms,
                    reason: r.pause_reason,
                },
            );
        }
        let observed = r
            .status
            .as_deref()
            .and_then(Status::parse)
            .map(|status| Observed {
                status,
                failures: r
                    .consecutive_failures
                    .and_then(|f| u32::try_from(f).ok())
                    .unwrap_or(0),
                last_change: r
                    .change_ts
                    .map(|ts| (ts, r.change_from.as_deref().and_then(Status::parse))),
            });
        pauses.sensor_info.insert(
            r.id,
            SensorInfo {
                device_id: r.device_id,
                name: r.name,
                device: r.device_name,
                setting: setting(
                    &r.dependency,
                    r.dependency_sensor_id,
                    r.dependency_delay_secs,
                ),
                master: r.dependency_master,
                observed,
            },
        );
    }
    crate::storage::schedules::load_into(conn, &mut pauses, now_ms).await?;
    Ok(pauses)
}

/// [`load`] on a connection of the pool.
pub async fn load_all(pool: &SqlitePool, now_ms: i64) -> Result<Pauses> {
    load(&mut *pool.acquire().await?, now_ms).await
}

fn log_column(target: Target) -> &'static str {
    match target {
        Target::Group(_) => "group_id",
        Target::Device(_) => "device_id",
        Target::Sensor(_) => "sensor_id",
    }
}

async fn log(
    conn: &mut SqliteConnection,
    ts_ms: i64,
    action: Action,
    target: Target,
    pause: Option<&Pause>,
    user: Option<&str>,
) -> Result<()> {
    sqlx::query(&format!(
        "INSERT INTO pause_log (ts_ms, action, {}, until_ms, reason, user_name) VALUES (?, ?, ?, ?, ?, ?)",
        log_column(target)
    ))
    .bind(ts_ms)
    .bind(action.as_str())
    .bind(target.id())
    .bind(pause.and_then(|p| p.until_ms))
    .bind(pause.map_or("", |p| p.reason.as_str()))
    .bind(user)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "DELETE FROM pause_log WHERE id <= (SELECT id FROM pause_log ORDER BY id DESC LIMIT 1 OFFSET ?)",
    )
    .bind(LOG_KEPT)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Marks every polled sensor that is paused now (by itself, its device, a group, a maintenance window
/// or a dependency) but not stored as paused yet, see [`crate::storage::status::mark_paused`], and logs
/// when a sensor starts or stops pausing others through dependencies and when a window opens and closes. Marking one sensor can pause the next in a chain,
/// so it repeats until nothing changes.
pub(crate) async fn sync(conn: &mut SqliteConnection, now_ms: i64) -> Result<()> {
    let mut pauses = load(conn, now_ms).await?;
    for _ in 0..=pauses.sensor_info.len() {
        let due: Vec<(i64, Option<Status>)> = pauses
            .sensor_info
            .iter()
            .filter_map(|(id, info)| {
                // A sensor never polled has no state to end; it shows as paused anyway.
                let stored = info.observed.as_ref()?.status;
                (stored != Status::Paused && pauses.sensor(*id, info.device_id).is_some())
                    .then_some((*id, Some(stored)))
            })
            .collect();
        if due.is_empty() {
            break;
        }
        for (id, from) in due {
            crate::storage::status::mark_paused(conn, id, from, now_ms).await?;
        }
        pauses = load(conn, now_ms).await?;
    }
    log_dependencies(conn, &pauses, now_ms).await?;
    crate::storage::schedules::log_windows(conn, &pauses, now_ms).await
}

/// Sensors whose last dependency entry says they pause others.
async fn active_dependencies(conn: &mut SqliteConnection) -> Result<HashSet<i64>> {
    let active: Vec<i64> = sqlx::query_scalar(
        "SELECT sensor_id FROM dependency_log l \
         WHERE id = (SELECT MAX(id) FROM dependency_log WHERE sensor_id = l.sensor_id) \
         AND action = 'active'",
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(active.into_iter().collect())
}

async fn log_dependencies(conn: &mut SqliteConnection, pauses: &Pauses, now_ms: i64) -> Result<()> {
    let roots = pauses.dependency_roots();
    let active = active_dependencies(conn).await?;
    let mut started: Vec<_> = roots
        .iter()
        .filter(|(id, _)| !active.contains(id))
        .collect();
    started.sort_by_key(|(id, _)| **id);
    let mut ended: Vec<_> = active.iter().filter(|id| !roots.contains_key(id)).collect();
    ended.sort_unstable();
    for (id, affected) in started {
        write_dependency_log(
            conn,
            now_ms,
            *id,
            "active",
            affected.sensors,
            affected.devices.len(),
        )
        .await?;
    }
    for id in ended {
        write_dependency_log(conn, now_ms, *id, "ended", 0, 0).await?;
    }
    Ok(())
}

async fn write_dependency_log(
    conn: &mut SqliteConnection,
    ts_ms: i64,
    sensor_id: i64,
    action: &str,
    sensors: usize,
    devices: usize,
) -> Result<()> {
    let count = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
    sqlx::query(
        "INSERT INTO dependency_log (ts_ms, sensor_id, action, sensors, devices) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(ts_ms)
    .bind(sensor_id)
    .bind(action)
    .bind(count(sensors))
    .bind(count(devices))
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "DELETE FROM dependency_log WHERE id <= (SELECT id FROM dependency_log ORDER BY id DESC LIMIT 1 OFFSET ?)",
    )
    .bind(LOG_KEPT)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// [`sync`] in its own transaction: after polls that changed a status and periodically, so that
/// dependent sensors show as paused right away and delays end.
pub async fn sync_all(pool: &SqlitePool, now_ms: i64) -> Result<()> {
    let mut tx = pool.begin().await?;
    sync(&mut tx, now_ms).await?;
    tx.commit().await?;
    Ok(())
}

/// Stores the dependency setting of an object; for a sensor also whether it is the master of its
/// device (only one per device). Refused if a sensor would then depend on itself, or if the sensor
/// depended on does not exist. Errors with `NotFound` if the object is missing.
pub async fn set_dependency(
    pool: &SqlitePool,
    target: Target,
    setting: Setting,
    master: Option<bool>,
    now_ms: i64,
) -> std::result::Result<(), DependencyError> {
    let mut tx = pool.begin().await?;
    let pauses = load(&mut tx, now_ms).await?;
    let exists = match target {
        Target::Group(id) => pauses.groups.contains_key(&id),
        Target::Device(id) => pauses.devices.contains_key(&id),
        Target::Sensor(id) => pauses.sensor_info(id).is_some(),
    };
    if !exists {
        return Err(Error::NotFound.into());
    }
    if let Some(sensor) = setting.dependency.sensor_id()
        && pauses.sensor_info(sensor).is_none()
    {
        return Err(DependencyError::Invalid(Msg::new(
            "dependency-sensor-missing",
        )));
    }
    if pauses.with_setting(target, setting, master).has_cycle() {
        return Err(DependencyError::Invalid(Msg::new("dependency-cycle")));
    }
    let table = match target {
        Target::Group(_) => "groups",
        Target::Device(_) => "devices",
        Target::Sensor(_) => "sensors",
    };
    sqlx::query(&format!(
        "UPDATE {table} SET dependency = ?, dependency_sensor_id = ?, dependency_delay_secs = ? WHERE id = ?"
    ))
    .bind(setting.dependency.as_str())
    .bind(setting.dependency.sensor_id())
    .bind(setting.delay_secs.map(i64::from))
    .bind(target.id())
    .execute(&mut *tx)
    .await?;
    if let (Target::Sensor(id), Some(master)) = (target, master) {
        if master {
            sqlx::query(
                "UPDATE sensors SET dependency_master = 0 \
                 WHERE device_id = (SELECT device_id FROM sensors WHERE id = ?)",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("UPDATE sensors SET dependency_master = ? WHERE id = ?")
            .bind(master)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    sync(&mut tx, now_ms).await?;
    tx.commit().await?;
    Ok(())
}

/// Makes a sensor the master of its device if the device has none (a template applied to it).
pub async fn set_master_if_none(pool: &SqlitePool, device_id: i64, sensor_id: i64) -> Result<()> {
    sqlx::query(
        "UPDATE sensors SET dependency_master = 1 WHERE id = ? AND device_id = ? \
         AND NOT EXISTS (SELECT 1 FROM sensors WHERE device_id = ? AND dependency_master = 1)",
    )
    .bind(sensor_id)
    .bind(device_id)
    .bind(device_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Why a dependency setting was not stored.
#[derive(Debug)]
pub enum DependencyError {
    /// Explained to the user.
    Invalid(Msg),
    Storage(Error),
}

impl From<Error> for DependencyError {
    fn from(e: Error) -> Self {
        Self::Storage(e)
    }
}

impl From<sqlx::Error> for DependencyError {
    fn from(e: sqlx::Error) -> Self {
        Self::Storage(e.into())
    }
}

/// One entry of the dependency log with the names of the sensor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyEntry {
    pub ts_ms: i64,
    /// `true` when the sensor started pausing others, `false` when that ended.
    pub active: bool,
    pub sensor_id: i64,
    pub sensor: String,
    pub device: (i64, String),
    pub group_id: Option<i64>,
    pub sensors: i64,
    pub devices: i64,
    /// For a start: when the next entry of the same sensor (its end) was written.
    pub end_ms: Option<i64>,
}

type DependencyRow = (
    i64,
    String,
    i64,
    String,
    i64,
    String,
    Option<i64>,
    i64,
    i64,
    Option<i64>,
);

/// The dependency log, newest first.
pub async fn dependency_entries(pool: &SqlitePool) -> Result<Vec<DependencyEntry>> {
    let rows: Vec<DependencyRow> = sqlx::query_as(
        "SELECT l.ts_ms, l.action, l.sensor_id, s.name, d.id, d.name, d.group_id, l.sensors, l.devices, \
         LEAD(l.ts_ms) OVER (PARTITION BY l.sensor_id ORDER BY l.ts_ms, l.id) \
         FROM dependency_log l JOIN sensors s ON s.id = l.sensor_id JOIN devices d ON d.id = s.device_id \
         ORDER BY l.ts_ms DESC, l.id DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(
                ts_ms,
                action,
                sensor_id,
                sensor,
                device_id,
                device,
                group_id,
                sensors,
                devices,
                end_ms,
            )| {
                DependencyEntry {
                    ts_ms,
                    active: action == "active",
                    sensor_id,
                    sensor,
                    device: (device_id, device),
                    group_id,
                    sensors,
                    devices,
                    end_ms,
                }
            },
        )
        .collect())
}

/// Pauses an object (again: with the new end and reason). Errors with `NotFound` if it is missing.
pub async fn pause(
    pool: &SqlitePool,
    target: Target,
    pause: &Pause,
    user: &str,
    now_ms: i64,
) -> Result<()> {
    let sql = match target {
        Target::Sensor(_) => {
            "UPDATE sensors SET enabled = 0, pause_until_ms = ?, pause_reason = ? WHERE id = ?"
        }
        Target::Device(_) => {
            "UPDATE devices SET paused = 1, pause_until_ms = ?, pause_reason = ? WHERE id = ?"
        }
        Target::Group(_) => {
            "UPDATE groups SET paused = 1, pause_until_ms = ?, pause_reason = ? WHERE id = ?"
        }
    };
    let mut tx = pool.begin().await?;
    let done = sqlx::query(sql)
        .bind(pause.until_ms)
        .bind(&pause.reason)
        .bind(target.id())
        .execute(&mut *tx)
        .await?;
    if done.rows_affected() == 0 {
        return Err(Error::NotFound);
    }
    log(
        &mut tx,
        now_ms,
        Action::Paused,
        target,
        Some(pause),
        Some(user),
    )
    .await?;
    sync(&mut tx, now_ms).await?;
    tx.commit().await?;
    Ok(())
}

const RESUME_SENSOR: &str = "UPDATE sensors SET enabled = 1, pause_until_ms = NULL, pause_reason = '' WHERE id = ? AND enabled = 0";
const RESUME_DEVICE: &str = "UPDATE devices SET paused = 0, pause_until_ms = NULL, pause_reason = '' WHERE id = ? AND paused = 1";
const RESUME_GROUP: &str = "UPDATE groups SET paused = 0, pause_until_ms = NULL, pause_reason = '' WHERE id = ? AND paused = 1";

fn resume_sql(target: Target) -> &'static str {
    match target {
        Target::Sensor(_) => RESUME_SENSOR,
        Target::Device(_) => RESUME_DEVICE,
        Target::Group(_) => RESUME_GROUP,
    }
}

/// Ends the own pause of an object; `false` if it was not paused by itself. The sensors keep their
/// stored "paused" state until their next poll, which then starts afresh.
pub async fn resume(pool: &SqlitePool, target: Target, user: &str, now_ms: i64) -> Result<bool> {
    let mut tx = pool.begin().await?;
    let done = sqlx::query(resume_sql(target))
        .bind(target.id())
        .execute(&mut *tx)
        .await?;
    if done.rows_affected() == 0 {
        return Ok(false);
    }
    log(&mut tx, now_ms, Action::Resumed, target, None, Some(user)).await?;
    tx.commit().await?;
    Ok(true)
}

/// Ends the timed pauses whose end has passed and logs them at their end. Returns how many ended.
pub async fn expire(pool: &SqlitePool, now_ms: i64) -> Result<usize> {
    let mut tx = pool.begin().await?;
    let mut ended = 0;
    for (select, make) in [
        (
            "SELECT id, pause_until_ms FROM sensors WHERE enabled = 0 AND pause_until_ms <= ?",
            Target::Sensor as fn(i64) -> Target,
        ),
        (
            "SELECT id, pause_until_ms FROM devices WHERE paused = 1 AND pause_until_ms <= ?",
            Target::Device,
        ),
        (
            "SELECT id, pause_until_ms FROM groups WHERE paused = 1 AND pause_until_ms <= ?",
            Target::Group,
        ),
    ] {
        let due: Vec<(i64, i64)> = sqlx::query_as(select)
            .bind(now_ms)
            .fetch_all(&mut *tx)
            .await?;
        for (id, until_ms) in due {
            let target = make(id);
            sqlx::query(resume_sql(target))
                .bind(id)
                .execute(&mut *tx)
                .await?;
            log(&mut tx, until_ms, Action::Expired, target, None, None).await?;
            ended += 1;
        }
    }
    tx.commit().await?;
    Ok(ended)
}

/// One entry of the pause log with the names of what it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub ts_ms: i64,
    pub action: Action,
    pub target: Target,
    pub sensor: Option<String>,
    /// The device itself, or the sensor's device.
    pub device: Option<(i64, String)>,
    pub group: Option<String>,
    /// For a pause: when it was to end (`None` = until resumed).
    pub until_ms: Option<i64>,
    pub reason: String,
    /// `None` for an expired pause.
    pub user: Option<String>,
    /// For a pause: when the next entry of the same object (its end) was written.
    pub end_ms: Option<i64>,
}

#[derive(FromRow)]
struct LogRow {
    ts_ms: i64,
    action: String,
    sensor_id: Option<i64>,
    device_id: Option<i64>,
    group_id: Option<i64>,
    until_ms: Option<i64>,
    reason: String,
    user_name: Option<String>,
    sensor_name: Option<String>,
    device_ref: Option<i64>,
    device_name: Option<String>,
    group_name: Option<String>,
    end_ms: Option<i64>,
}

/// The pause log, newest first.
pub async fn log_entries(pool: &SqlitePool) -> Result<Vec<LogEntry>> {
    let rows = sqlx::query_as::<_, LogRow>(
        "SELECT l.ts_ms, l.action, l.sensor_id, l.device_id, l.group_id, l.until_ms, l.reason, l.user_name, \
         s.name AS sensor_name, d.id AS device_ref, d.name AS device_name, g.name AS group_name, \
         LEAD(l.ts_ms) OVER (PARTITION BY l.sensor_id, l.device_id, l.group_id ORDER BY l.ts_ms, l.id) AS end_ms \
         FROM pause_log l \
         LEFT JOIN sensors s ON s.id = l.sensor_id \
         LEFT JOIN devices d ON d.id = COALESCE(l.device_id, s.device_id) \
         LEFT JOIN groups g ON g.id = l.group_id \
         ORDER BY l.ts_ms DESC, l.id DESC",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            let target = match (r.sensor_id, r.device_id, r.group_id) {
                (Some(id), _, _) => Target::Sensor(id),
                (_, Some(id), _) => Target::Device(id),
                (_, _, Some(id)) => Target::Group(id),
                _ => return Err(Error::Internal("pause log entry without target".into())),
            };
            Ok(LogEntry {
                ts_ms: r.ts_ms,
                action: Action::parse(&r.action)
                    .ok_or_else(|| Error::Internal("invalid pause action in database".into()))?,
                target,
                sensor: r.sensor_name,
                device: r.device_ref.zip(r.device_name),
                group: r.group_name,
                until_ms: r.until_ms,
                reason: r.reason,
                user: r.user_name,
                end_ms: r.end_ms,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{DeviceInput, Polling};
    use crate::pause::Source;
    use crate::sensors::spec::SensorSpec;
    use crate::status::Assessment;
    use crate::storage::{self, devices, groups, sensors, status};

    struct Env {
        pool: SqlitePool,
        group: i64,
        device: i64,
        sensor: i64,
        _dir: tempfile::TempDir,
    }

    async fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let pool = storage::connect(&dir.path().join("p.db")).await.unwrap();
        storage::migrate(&pool).await.unwrap();
        let group = groups::create_group(&pool, "Netzwerk", None).await.unwrap();
        let device = devices::save_device(
            &pool,
            None,
            DeviceInput {
                kind: Default::default(),
                name: "Switch".into(),
                host: "192.0.2.1".into(),
                group_id: Some(group),
                profile_id: None,
                polling: Polling::default(),
            },
        )
        .await
        .unwrap();
        let sensor = sensors::create_sensor(&pool, device, "Ping", &SensorSpec::Ping)
            .await
            .unwrap();
        Env {
            pool,
            group,
            device,
            sensor,
            _dir: dir,
        }
    }

    fn until(until_ms: Option<i64>, reason: &str) -> Pause {
        Pause {
            until_ms,
            reason: reason.to_owned(),
        }
    }

    async fn stored(env: &Env) -> Option<Status> {
        status::get_state(&env.pool, env.sensor)
            .await
            .unwrap()
            .map(|s| s.status)
    }

    #[tokio::test]
    async fn pausing_a_group_pauses_its_sensors_and_ends_their_outage() {
        let env = env().await;
        for ts in [1_000, 2_000] {
            status::record_poll(
                &env.pool,
                env.sensor,
                ts,
                &[],
                &Assessment::Failed(crate::i18n::Msg::Text("weg".into())),
            )
            .await
            .unwrap();
        }
        assert_eq!(stored(&env).await, Some(Status::Down));

        let group = Target::Group(env.group);
        pause(&env.pool, group, &until(None, "Umbau"), "anna", 3_000)
            .await
            .unwrap();
        let pauses = load_all(&env.pool, 3_000).await.unwrap();
        let why = pauses.sensor(env.sensor, env.device).unwrap();
        assert_eq!(why.source, Source::Group("Netzwerk".into()));
        assert_eq!(stored(&env).await, Some(Status::Paused));
        let events = status::events_of_sensor(&env.pool, env.sensor, 10)
            .await
            .unwrap();
        assert_eq!(
            (events[0].from, events[0].to, events[0].ts_ms),
            (Some(Status::Down), Status::Paused, 3_000),
            "the outage ends when the pause starts"
        );

        assert!(resume(&env.pool, group, "anna", 4_000).await.unwrap());
        assert!(
            !resume(&env.pool, group, "anna", 4_500).await.unwrap(),
            "not paused any more"
        );
        assert!(
            load_all(&env.pool, 4_000)
                .await
                .unwrap()
                .sensor(env.sensor, env.device)
                .is_none()
        );
        // The first poll after the pause starts afresh: a success is no recovery to mail about.
        let change = status::record_poll(
            &env.pool,
            env.sensor,
            5_000,
            &[],
            &Assessment::Healthy(crate::i18n::Msg::Text("ok".into())),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!((change.from, change.to), (Some(Status::Paused), Status::Up));
        assert!(!crate::alerting::is_alertworthy(&change, true));

        let log = log_entries(&env.pool).await.unwrap();
        let shape: Vec<_> = log
            .iter()
            .map(|e| (e.action, e.target, e.user.as_deref(), e.end_ms))
            .collect();
        assert_eq!(
            shape,
            [
                (Action::Resumed, group, Some("anna"), None),
                (Action::Paused, group, Some("anna"), Some(4_000)),
            ]
        );
        assert_eq!(log[1].reason, "Umbau");
        assert_eq!(log[1].group.as_deref(), Some("Netzwerk"));
    }

    #[tokio::test]
    async fn timed_pauses_end_by_themselves() {
        let env = env().await;
        let sensor = Target::Sensor(env.sensor);
        pause(&env.pool, sensor, &until(Some(10_000), ""), "anna", 1_000)
            .await
            .unwrap();
        pause(
            &env.pool,
            Target::Device(env.device),
            &until(Some(20_000), "Wartung"),
            "anna",
            1_000,
        )
        .await
        .unwrap();
        assert_eq!(expire(&env.pool, 9_999).await.unwrap(), 0);
        let pauses = load_all(&env.pool, 15_000).await.unwrap();
        assert_eq!(
            pauses.sensor(env.sensor, env.device).unwrap().source,
            Source::Device,
            "an ended pause no longer counts even before expire ran"
        );
        assert_eq!(expire(&env.pool, 25_000).await.unwrap(), 2);
        assert!(
            load_all(&env.pool, 25_000)
                .await
                .unwrap()
                .sensor(env.sensor, env.device)
                .is_none()
        );
        let record = sensors::get_sensor(&env.pool, env.sensor)
            .await
            .unwrap()
            .unwrap();
        assert!(record.enabled);
        let log = log_entries(&env.pool).await.unwrap();
        let expired: Vec<_> = log
            .iter()
            .filter(|e| e.action == Action::Expired)
            .map(|e| (e.target, e.ts_ms, e.user.clone()))
            .collect();
        assert_eq!(
            expired,
            [
                (Target::Device(env.device), 20_000, None),
                (Target::Sensor(env.sensor), 10_000, None)
            ]
        );
        assert_eq!(
            log.iter()
                .find(|e| e.target == sensor && e.action == Action::Paused)
                .unwrap()
                .device,
            Some((env.device, "Switch".to_owned()))
        );
    }

    async fn poll(env: &Env, sensor: i64, ts: i64, ok: bool) {
        let assessment = if ok {
            Assessment::Healthy(crate::i18n::Msg::Text("ok".into()))
        } else {
            Assessment::Failed(crate::i18n::Msg::Text("weg".into()))
        };
        status::record_poll(&env.pool, sensor, ts, &[], &assessment)
            .await
            .unwrap();
    }

    fn on(sensor: i64, delay_secs: Option<u32>) -> Setting {
        Setting {
            dependency: Dependency::Sensor(sensor),
            delay_secs,
        }
    }

    #[tokio::test]
    async fn dependencies_pause_mark_and_log_once_per_outage() {
        let env = env().await;
        // A router outside the group; the group depends on its ping, with a delay of 30 s.
        let router = devices::save_device(
            &env.pool,
            None,
            DeviceInput {
                kind: Default::default(),
                name: "Router".into(),
                host: "192.0.2.254".into(),
                group_id: None,
                profile_id: None,
                polling: Polling::default(),
            },
        )
        .await
        .unwrap();
        let router_ping = sensors::create_sensor(&env.pool, router, "Ping", &SensorSpec::Ping)
            .await
            .unwrap();
        poll(&env, router_ping, 1_000, true).await;
        poll(&env, env.sensor, 1_000, true).await;
        set_dependency(
            &env.pool,
            Target::Group(env.group),
            on(router_ping, Some(30)),
            None,
            2_000,
        )
        .await
        .unwrap();
        let pauses = load_all(&env.pool, 2_000).await.unwrap();
        assert_eq!(
            pauses.setting(Target::Group(env.group)),
            on(router_ping, Some(30))
        );
        assert!(pauses.sensor(env.sensor, env.device).is_none());

        // The router goes down: the switch ping is paused, marked and the start is logged.
        poll(&env, router_ping, 3_000, false).await;
        poll(&env, router_ping, 4_000, false).await;
        sync_all(&env.pool, 4_000).await.unwrap();
        assert_eq!(stored(&env).await, Some(Status::Paused));
        sync_all(&env.pool, 5_000).await.unwrap();
        let log = dependency_entries(&env.pool).await.unwrap();
        assert_eq!(log.len(), 1, "logged once, not with every sync: {log:?}");
        assert_eq!(
            (
                log[0].active,
                log[0].sensor_id,
                log[0].sensors,
                log[0].devices
            ),
            (true, router_ping, 1, 1)
        );
        assert_eq!(log[0].device, (router, "Router".to_owned()));

        // Back up: still paused for the delay, then running again and the end is logged.
        poll(&env, router_ping, 10_000, true).await;
        let pauses = load_all(&env.pool, 20_000).await.unwrap();
        let why = pauses.sensor(env.sensor, env.device).unwrap();
        assert_eq!(why.pause.until_ms, Some(40_000));
        sync_all(&env.pool, 20_000).await.unwrap();
        assert!(dependency_entries(&env.pool).await.unwrap()[0].active);
        sync_all(&env.pool, 40_000).await.unwrap();
        let log = dependency_entries(&env.pool).await.unwrap();
        assert_eq!(
            log.iter()
                .map(|e| (e.active, e.ts_ms, e.end_ms))
                .collect::<Vec<_>>(),
            [(false, 40_000, None), (true, 4_000, Some(40_000))]
        );
        assert!(
            load_all(&env.pool, 40_000)
                .await
                .unwrap()
                .sensor(env.sensor, env.device)
                .is_none()
        );
    }

    #[tokio::test]
    async fn dependency_settings_are_checked_before_they_are_stored() {
        let env = env().await;
        let uptime = sensors::create_sensor(&env.pool, env.device, "Uptime", &SensorSpec::Uptime)
            .await
            .unwrap();
        // The group depends on the ping of a device inside it: fine, the ping never depends on itself.
        set_dependency(
            &env.pool,
            Target::Group(env.group),
            on(env.sensor, None),
            None,
            1,
        )
        .await
        .unwrap();
        // The ping depending on the uptime that depends on the ping is a cycle.
        let cycle = set_dependency(
            &env.pool,
            Target::Sensor(env.sensor),
            on(uptime, None),
            None,
            1,
        )
        .await;
        assert!(
            matches!(cycle, Err(DependencyError::Invalid(ref m)) if *m == Msg::new("dependency-cycle"))
        );
        let missing = set_dependency(
            &env.pool,
            Target::Device(env.device),
            on(999, None),
            None,
            1,
        )
        .await;
        assert!(matches!(missing, Err(DependencyError::Invalid(_))));
        let gone = set_dependency(&env.pool, Target::Device(99), Setting::default(), None, 1).await;
        assert!(matches!(
            gone,
            Err(DependencyError::Storage(Error::NotFound))
        ));

        // The uptime as master while the group depends on the ping would be a cycle as well.
        let cycle = set_dependency(
            &env.pool,
            Target::Sensor(uptime),
            Setting::default(),
            Some(true),
            1,
        )
        .await;
        assert!(matches!(cycle, Err(DependencyError::Invalid(_))));

        // One master per device.
        set_dependency(
            &env.pool,
            Target::Group(env.group),
            Setting::default(),
            None,
            1,
        )
        .await
        .unwrap();
        set_dependency(
            &env.pool,
            Target::Sensor(env.sensor),
            Setting::default(),
            Some(true),
            1,
        )
        .await
        .unwrap();
        set_dependency(
            &env.pool,
            Target::Sensor(uptime),
            Setting::default(),
            Some(true),
            1,
        )
        .await
        .unwrap();
        let pauses = load_all(&env.pool, 1).await.unwrap();
        assert_eq!(pauses.master_of(env.device), Some(uptime));
        set_master_if_none(&env.pool, env.device, env.sensor)
            .await
            .unwrap();
        assert_eq!(
            load_all(&env.pool, 1).await.unwrap().master_of(env.device),
            Some(uptime),
            "a template does not replace the master"
        );
        set_dependency(
            &env.pool,
            Target::Sensor(env.sensor),
            Setting::default(),
            Some(true),
            1,
        )
        .await
        .unwrap();
        set_dependency(
            &env.pool,
            Target::Group(env.group),
            on(env.sensor, None),
            None,
            1,
        )
        .await
        .unwrap();

        // Deleting the sensor depended on makes the group inherit again.
        sensors::delete_sensor(&env.pool, env.sensor).await.unwrap();
        let pauses = load_all(&env.pool, 1).await.unwrap();
        assert_eq!(pauses.setting(Target::Group(env.group)), Setting::default());
    }

    #[tokio::test]
    async fn missing_objects_are_not_found() {
        let env = env().await;
        let missing = pause(&env.pool, Target::Device(99), &until(None, ""), "anna", 1).await;
        assert!(matches!(missing, Err(Error::NotFound)));
        assert!(
            !resume(&env.pool, Target::Sensor(99), "anna", 1)
                .await
                .unwrap()
        );
    }
}
