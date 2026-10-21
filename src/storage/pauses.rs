//! Persistence of pauses (see [`crate::pause`]): the pause columns of `sensors`, `devices` and
//! `groups` and the log `pause_log`.

use sqlx::{FromRow, SqliteConnection, SqlitePool};

use crate::error::{Error, Result};
use crate::pause::{Pause, Pauses, Target};
use crate::status::Status;

/// Entries kept in `pause_log`; older ones are deleted whenever a new one is written.
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

/// id, name, parent, paused, until, reason.
type GroupRow = (i64, String, Option<i64>, bool, Option<i64>, String);
/// id, group, paused, until, reason.
type DeviceRow = (i64, Option<i64>, bool, Option<i64>, String);

/// All pauses as of `now_ms`.
pub async fn load(conn: &mut SqliteConnection, now_ms: i64) -> Result<Pauses> {
    let mut pauses = Pauses {
        now_ms,
        ..Pauses::default()
    };
    let groups: Vec<GroupRow> = sqlx::query_as(
        "SELECT id, name, parent_id, paused, pause_until_ms, pause_reason FROM groups",
    )
    .fetch_all(&mut *conn)
    .await?;
    for (id, name, parent, paused, until, reason) in groups {
        pauses
            .groups
            .insert(id, (name, parent, own(paused, until, reason)));
    }
    let devices: Vec<DeviceRow> =
        sqlx::query_as("SELECT id, group_id, paused, pause_until_ms, pause_reason FROM devices")
            .fetch_all(&mut *conn)
            .await?;
    for (id, group, paused, until, reason) in devices {
        pauses
            .devices
            .insert(id, (group, own(paused, until, reason)));
    }
    let sensors: Vec<(i64, Option<i64>, String)> =
        sqlx::query_as("SELECT id, pause_until_ms, pause_reason FROM sensors WHERE enabled = 0")
            .fetch_all(&mut *conn)
            .await?;
    for (id, until_ms, reason) in sensors {
        pauses.sensors.insert(id, Pause { until_ms, reason });
    }
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

/// Marks every sensor that is paused now (by itself, its device or a group) but not stored as
/// paused yet, see [`crate::storage::status::mark_paused`].
async fn mark_paused_sensors(conn: &mut SqliteConnection, now_ms: i64) -> Result<()> {
    let pauses = load(conn, now_ms).await?;
    let sensors: Vec<(i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT s.id, s.device_id, st.status FROM sensors s \
         LEFT JOIN sensor_status st ON st.sensor_id = s.id",
    )
    .fetch_all(&mut *conn)
    .await?;
    for (id, device, status) in sensors {
        let status = status.as_deref().and_then(Status::parse);
        if status != Some(Status::Paused) && pauses.sensor(id, device).is_some() {
            crate::storage::status::mark_paused(conn, id, status, now_ms).await?;
        }
    }
    Ok(())
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
    mark_paused_sensors(&mut tx, now_ms).await?;
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
