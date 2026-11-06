//! Persistence of sensor states and the status-change event log.

use std::collections::HashMap;

use sqlx::{FromRow, SqlitePool};

use crate::error::{Error, Result};
use crate::i18n::Msg;
use crate::status::{Assessment, SensorState, Status, advance};

use super::sensors::Sample;

/// A status transition, e.g. for alerting. `from` is `None` for the first poll of a sensor.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusChange {
    pub sensor_id: i64,
    pub from: Option<Status>,
    pub to: Status,
    pub message: Msg,
    pub ts_ms: i64,
}

/// Status changes kept per sensor in `events`; older ones of that sensor are deleted whenever a
/// new one is written. A sensor that flaps all day therefore never pushes out the changes of the
/// quiet ones.
pub const EVENTS_KEPT_PER_SENSOR: i64 = 1000;

/// Entries the problem log shows (`/log`), the newest ones.
pub const LOG_ENTRIES: i64 = 500;

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: i64,
    pub ts_ms: i64,
    pub sensor_id: i64,
    pub from: Option<Status>,
    pub to: Status,
    pub message: Msg,
}

#[derive(FromRow)]
struct StateRow {
    sensor_id: i64,
    status: String,
    since_ms: i64,
    last_poll_ms: i64,
    message: String,
    consecutive_failures: i64,
}

fn parse_status(s: &str) -> Result<Status> {
    Status::parse(s).ok_or_else(|| Error::Internal("invalid status in database".to_owned()))
}

impl TryFrom<StateRow> for (i64, SensorState) {
    type Error = Error;

    fn try_from(r: StateRow) -> Result<Self> {
        Ok((
            r.sensor_id,
            SensorState {
                status: parse_status(&r.status)?,
                since_ms: r.since_ms,
                last_poll_ms: r.last_poll_ms,
                message: Msg::decode(&r.message),
                consecutive_failures: u32::try_from(r.consecutive_failures)
                    .map_err(|_| Error::Internal("invalid failure count in database".to_owned()))?,
            },
        ))
    }
}

const STATE_SELECT: &str = "SELECT sensor_id, status, since_ms, last_poll_ms, message, consecutive_failures \
                            FROM sensor_status";

pub async fn get_state(pool: &SqlitePool, sensor_id: i64) -> Result<Option<SensorState>> {
    sqlx::query_as::<_, StateRow>(&format!("{STATE_SELECT} WHERE sensor_id = ?"))
        .bind(sensor_id)
        .fetch_optional(pool)
        .await?
        .map(|r| <(i64, SensorState)>::try_from(r).map(|(_, s)| s))
        .transpose()
}

/// States of all sensors by sensor id.
pub async fn list_states(pool: &SqlitePool) -> Result<HashMap<i64, SensorState>> {
    sqlx::query_as::<_, StateRow>(STATE_SELECT)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(<(i64, SensorState)>::try_from)
        .collect()
}

/// Stores the samples of one poll, updates the sensor state and logs a status change, all in one
/// transaction. Returns the change if the status differs from before (or on the first poll).
pub async fn record_poll(
    pool: &SqlitePool,
    sensor_id: i64,
    ts_ms: i64,
    samples: &[Sample],
    assessment: &Assessment,
) -> Result<Option<StatusChange>> {
    let mut tx = pool.begin().await?;
    for s in samples {
        sqlx::query("INSERT OR IGNORE INTO samples (channel_id, ts, value) VALUES (?, ?, ?)")
            .bind(s.channel_id)
            .bind(s.ts_ms)
            .bind(s.value)
            .execute(&mut *tx)
            .await?;
    }
    let prev = sqlx::query_as::<_, StateRow>(&format!("{STATE_SELECT} WHERE sensor_id = ?"))
        .bind(sensor_id)
        .fetch_optional(&mut *tx)
        .await?
        .map(<(i64, SensorState)>::try_from)
        .transpose()?
        .map(|(_, s)| s);
    // The time since the poll before belongs to the status it had (for the availability).
    if let Some(p) = &prev {
        crate::storage::availability::count(&mut tx, sensor_id, p.status, p.last_poll_ms, ts_ms)
            .await?;
    }
    // After a pause the sensor starts afresh: no failure count, no stale "recovered" from before.
    let basis = prev.as_ref().filter(|p| p.status != Status::Paused);
    let next = advance(basis, assessment, ts_ms);
    sqlx::query(
        "INSERT INTO sensor_status (sensor_id, status, since_ms, last_poll_ms, message, consecutive_failures) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT (sensor_id) DO UPDATE SET status = excluded.status, since_ms = excluded.since_ms, \
         last_poll_ms = excluded.last_poll_ms, message = excluded.message, \
         consecutive_failures = excluded.consecutive_failures",
    )
    .bind(sensor_id)
    .bind(next.status.as_str())
    .bind(next.since_ms)
    .bind(next.last_poll_ms)
    .bind(next.message.encode())
    .bind(i64::from(next.consecutive_failures))
    .execute(&mut *tx)
    .await?;
    // A failed poll writes no samples; the table of values shows it from here.
    if next.status != Status::Up {
        sqlx::query(
            "INSERT OR REPLACE INTO poll_problems (sensor_id, ts, status, message) VALUES (?, ?, ?, ?)",
        )
        .bind(sensor_id)
        .bind(ts_ms)
        .bind(next.status.as_str())
        .bind(next.message.encode())
        .execute(&mut *tx)
        .await?;
    }

    let from = prev.as_ref().map(|p| p.status);
    let change = (from != Some(next.status)).then(|| StatusChange {
        sensor_id,
        from,
        to: next.status,
        message: next.message.clone(),
        ts_ms,
    });
    if let Some(change) = &change {
        sqlx::query(
            "INSERT INTO events (ts_ms, sensor_id, from_status, to_status, message) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(change.ts_ms)
        .bind(sensor_id)
        .bind(change.from.map(Status::as_str))
        .bind(change.to.as_str())
        .bind(change.message.encode())
        .execute(&mut *tx)
        .await?;
        trim_events(&mut tx, sensor_id).await?;
    }
    tx.commit().await?;
    Ok(change)
}

/// Deletes all but the newest [`EVENTS_KEPT_PER_SENSOR`] events of one sensor.
async fn trim_events(conn: &mut sqlx::SqliteConnection, sensor_id: i64) -> Result<()> {
    sqlx::query(
        "DELETE FROM events WHERE sensor_id = ?1 AND id <= \
         (SELECT id FROM events WHERE sensor_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2)",
    )
    .bind(sensor_id)
    .bind(EVENTS_KEPT_PER_SENSOR)
    .execute(conn)
    .await?;
    Ok(())
}

/// Marks the stored state of a sensor as paused (keeping its last poll time) and logs the change.
/// Ends a running outage in the log at the moment of pausing and makes the first poll after
/// resuming start afresh (see [`record_poll`]).
pub(crate) async fn mark_paused(
    conn: &mut sqlx::SqliteConnection,
    sensor_id: i64,
    from: Option<Status>,
    ts_ms: i64,
) -> Result<()> {
    let message = Msg::new("status-paused").encode();
    sqlx::query(
        "INSERT INTO sensor_status (sensor_id, status, since_ms, last_poll_ms, message, consecutive_failures) \
         VALUES (?1, 'paused', ?2, ?2, ?3, 0) \
         ON CONFLICT (sensor_id) DO UPDATE SET status = 'paused', since_ms = excluded.since_ms, \
         message = excluded.message, consecutive_failures = 0",
    )
    .bind(sensor_id)
    .bind(ts_ms)
    .bind(&message)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO events (ts_ms, sensor_id, from_status, to_status, message) VALUES (?, ?, ?, 'paused', ?)",
    )
    .bind(ts_ms)
    .bind(sensor_id)
    .bind(from.map(Status::as_str))
    .bind(&message)
    .execute(&mut *conn)
    .await?;
    trim_events(conn, sensor_id).await
}

type EventRow = (i64, i64, i64, Option<String>, String, String);

fn event_from_row((id, ts_ms, sensor_id, from, to, message): EventRow) -> Result<Event> {
    Ok(Event {
        id,
        ts_ms,
        sensor_id,
        from: from.as_deref().map(parse_status).transpose()?,
        to: parse_status(&to)?,
        message: Msg::decode(&message),
    })
}

const EVENT_SELECT: &str =
    "SELECT id, ts_ms, sensor_id, from_status, to_status, message FROM events";

/// The newest events first.
pub async fn recent_events(pool: &SqlitePool, limit: u32) -> Result<Vec<Event>> {
    sqlx::query_as::<_, EventRow>(&format!(
        "{EVENT_SELECT} ORDER BY ts_ms DESC, id DESC LIMIT ?"
    ))
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(event_from_row)
    .collect()
}

/// `limit` events of one sensor, the newest first, after skipping the newest `offset` of them.
/// Ordered by `id` (the order they were written in), so a page can be found again by the id of
/// its first row (see [`events_newer_than`]).
pub async fn events_of_sensor(
    pool: &SqlitePool,
    sensor_id: i64,
    limit: u32,
    offset: u32,
) -> Result<Vec<Event>> {
    sqlx::query_as::<_, EventRow>(&format!(
        "{EVENT_SELECT} WHERE sensor_id = ? ORDER BY id DESC LIMIT ? OFFSET ?"
    ))
    .bind(sensor_id)
    .bind(i64::from(limit))
    .bind(i64::from(offset))
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(event_from_row)
    .collect()
}

/// How many events of a sensor are kept, and the time of the oldest one.
pub async fn event_totals(pool: &SqlitePool, sensor_id: i64) -> Result<(i64, Option<i64>)> {
    Ok(
        sqlx::query_as("SELECT COUNT(*), MIN(ts_ms) FROM events WHERE sensor_id = ?")
            .bind(sensor_id)
            .fetch_one(pool)
            .await?,
    )
}

/// How many events of a sensor were written after the one with `id`: the offset at which a page
/// that began with that event begins now. If the event is gone, the offset of the next older one.
pub async fn events_newer_than(pool: &SqlitePool, sensor_id: i64, id: i64) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE sensor_id = ? AND id > ?")
            .bind(sensor_id)
            .bind(id)
            .fetch_one(pool)
            .await?,
    )
}

/// An entry of the problem log: a change to warning or down, or back to up from one of them.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub event: Event,
    /// Time of the sensor's change before this one, if still kept.
    pub prev_ts_ms: Option<i64>,
    /// Time of the sensor's next change; `None` while this status lasts.
    pub next_ts_ms: Option<i64>,
}

type LogRow = (
    i64,
    i64,
    i64,
    Option<String>,
    String,
    String,
    Option<i64>,
    Option<i64>,
);

/// Warnings, outages and their ends, newest first, at most [`LOG_ENTRIES`]. `only` limits the log
/// to those sensors (the limit then counts within them, so a device's log is not crowded out by
/// busier sensors elsewhere).
pub async fn problem_log(pool: &SqlitePool, only: Option<&[i64]>) -> Result<Vec<LogEntry>> {
    let only = only.map(|ids| serde_json::to_string(ids).unwrap_or_else(|_| "[]".to_owned()));
    let rows: Vec<LogRow> = sqlx::query_as(
        "SELECT id, ts_ms, sensor_id, from_status, to_status, message, prev_ts, next_ts FROM ( \
               SELECT *, LAG(ts_ms) OVER w AS prev_ts, LEAD(ts_ms) OVER w AS next_ts FROM events \
               WINDOW w AS (PARTITION BY sensor_id ORDER BY ts_ms, id)) \
             WHERE (to_status IN ('warning', 'down') \
                OR (to_status = 'up' AND from_status IN ('warning', 'down'))) \
               AND (?1 IS NULL OR sensor_id IN (SELECT value FROM json_each(?1))) \
             ORDER BY ts_ms DESC, id DESC LIMIT ?2",
    )
    .bind(only)
    .bind(LOG_ENTRIES)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(id, ts, sensor, from, to, message, prev, next)| {
            Ok(LogEntry {
                event: event_from_row((id, ts, sensor, from, to, message))?,
                prev_ts_ms: prev,
                next_ts_ms: next,
            })
        })
        .collect()
}

/// The status of a sensor since `from_ms`: the status at `from_ms` (`None` if it had none yet) and
/// the changes after it as `(ts_ms, status)`, oldest first.
pub async fn status_history(
    pool: &SqlitePool,
    sensor_id: i64,
    from_ms: i64,
) -> Result<(Option<Status>, Vec<(i64, Status)>)> {
    let before: Option<String> = sqlx::query_scalar(
        "SELECT to_status FROM events WHERE sensor_id = ? AND ts_ms <= ? \
         ORDER BY ts_ms DESC, id DESC LIMIT 1",
    )
    .bind(sensor_id)
    .bind(from_ms)
    .fetch_optional(pool)
    .await?;
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT ts_ms, to_status FROM events WHERE sensor_id = ? AND ts_ms > ? ORDER BY ts_ms, id",
    )
    .bind(sensor_id)
    .bind(from_ms)
    .fetch_all(pool)
    .await?;
    let changes: Vec<(i64, Status)> = rows
        .into_iter()
        .map(|(ts, s)| Ok((ts, parse_status(&s)?)))
        .collect::<Result<_>>()?;
    let before = match before {
        Some(s) => Some(parse_status(&s)?),
        // Only the newest events are kept: the status before the first kept change, or the
        // current one if it already held at `from_ms`.
        None => status_before(pool, sensor_id, from_ms).await?,
    };
    Ok((before, changes))
}

async fn status_before(pool: &SqlitePool, sensor_id: i64, from_ms: i64) -> Result<Option<Status>> {
    let first: Option<Option<String>> = sqlx::query_scalar(
        "SELECT from_status FROM events WHERE sensor_id = ? AND ts_ms > ? ORDER BY ts_ms, id LIMIT 1",
    )
    .bind(sensor_id)
    .bind(from_ms)
    .fetch_optional(pool)
    .await?;
    if let Some(from) = first {
        return from.as_deref().map(parse_status).transpose();
    }
    let current: Option<String> = sqlx::query_scalar(
        "SELECT status FROM sensor_status WHERE sensor_id = ? AND since_ms <= ?",
    )
    .bind(sensor_id)
    .bind(from_ms)
    .fetch_optional(pool)
    .await?;
    current.as_deref().map(parse_status).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensors::spec::SensorSpec;
    use crate::storage::sensors::create_sensor;
    use crate::storage::{connect, migrate};

    async fn setup() -> (tempfile::TempDir, SqlitePool, i64) {
        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("t.db")).await.unwrap();
        migrate(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO credential_profiles (name, version, community_enc) VALUES ('p', 'v2c', x'01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let device: i64 = sqlx::query_scalar(
            "INSERT INTO devices (name, host, profile_id) VALUES ('sw', '192.0.2.1', 1) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let sensor = create_sensor(&pool, device, "Uptime", &SensorSpec::Uptime)
            .await
            .unwrap();
        (dir, pool, sensor)
    }

    /// Another sensor on the device of [`setup`].
    async fn add_sensor(pool: &SqlitePool, name: &str) -> i64 {
        create_sensor(pool, 1, name, &SensorSpec::Uptime)
            .await
            .unwrap()
    }

    fn ok() -> Assessment {
        Assessment::Healthy("OK".into())
    }

    fn fail() -> Assessment {
        Assessment::Failed("timeout".into())
    }

    #[tokio::test]
    async fn every_poll_counts_the_time_since_the_one_before_for_the_availability() {
        let (_dir, pool, sensor) = setup().await;
        record_poll(&pool, sensor, 1_000, &[], &ok()).await.unwrap();
        record_poll(&pool, sensor, 61_000, &[], &fail())
            .await
            .unwrap();
        record_poll(&pool, sensor, 121_000, &[], &fail())
            .await
            .unwrap(); // warning → down
        record_poll(&pool, sensor, 181_000, &[], &ok())
            .await
            .unwrap();
        // The service was stopped for an hour: not counted.
        record_poll(&pool, sensor, 3_781_000, &[], &ok())
            .await
            .unwrap();
        let totals = crate::storage::availability::totals(&pool, 0, 10_000_000, None)
            .await
            .unwrap();
        let t = totals[&sensor];
        assert_eq!((t.up_ms, t.warning_ms, t.down_ms), (60_000, 60_000, 60_000));
    }

    #[tokio::test]
    async fn first_poll_creates_state_and_a_first_event() {
        let (_dir, pool, sensor) = setup().await;
        let change = record_poll(&pool, sensor, 1_000, &[], &ok())
            .await
            .unwrap()
            .unwrap();
        assert_eq!((change.from, change.to), (None, Status::Up));
        let state = get_state(&pool, sensor).await.unwrap().unwrap();
        assert_eq!((state.status, state.since_ms), (Status::Up, 1_000));
        assert_eq!(recent_events(&pool, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn unchanged_status_logs_no_event_but_updates_the_poll_time() {
        let (_dir, pool, sensor) = setup().await;
        record_poll(&pool, sensor, 1_000, &[], &ok()).await.unwrap();
        let change = record_poll(&pool, sensor, 2_000, &[], &ok()).await.unwrap();
        assert!(change.is_none());
        let state = get_state(&pool, sensor).await.unwrap().unwrap();
        assert_eq!((state.since_ms, state.last_poll_ms), (1_000, 2_000));
        assert_eq!(recent_events(&pool, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn outage_and_recovery_are_logged_newest_first() {
        let (_dir, pool, sensor) = setup().await;
        record_poll(&pool, sensor, 1_000, &[], &ok()).await.unwrap();
        let warn = record_poll(&pool, sensor, 2_000, &[], &fail())
            .await
            .unwrap()
            .unwrap();
        assert_eq!((warn.from, warn.to), (Some(Status::Up), Status::Warning));
        let down = record_poll(&pool, sensor, 3_000, &[], &fail())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(down.to, Status::Down);
        assert!(
            record_poll(&pool, sensor, 4_000, &[], &fail())
                .await
                .unwrap()
                .is_none()
        );
        let up = record_poll(&pool, sensor, 5_000, &[], &ok())
            .await
            .unwrap()
            .unwrap();
        assert_eq!((up.from, up.to), (Some(Status::Down), Status::Up));

        let events = recent_events(&pool, 10).await.unwrap();
        let path: Vec<_> = events.iter().map(|e| e.to).collect();
        assert_eq!(
            path,
            [Status::Up, Status::Down, Status::Warning, Status::Up]
        );
        assert_eq!(recent_events(&pool, 2).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn the_status_history_starts_with_the_status_at_the_window_start() {
        let (_dir, pool, sensor) = setup().await;
        assert_eq!(
            status_history(&pool, sensor, 0).await.unwrap(),
            (None, Vec::new())
        );
        record_poll(&pool, sensor, 1_000, &[], &ok()).await.unwrap();
        record_poll(&pool, sensor, 2_000, &[], &fail())
            .await
            .unwrap();
        record_poll(&pool, sensor, 3_000, &[], &fail())
            .await
            .unwrap();
        record_poll(&pool, sensor, 5_000, &[], &ok()).await.unwrap();
        assert_eq!(
            status_history(&pool, sensor, 500).await.unwrap(),
            (
                None,
                vec![
                    (1_000, Status::Up),
                    (2_000, Status::Warning),
                    (3_000, Status::Down),
                    (5_000, Status::Up)
                ]
            )
        );
        assert_eq!(
            status_history(&pool, sensor, 4_000).await.unwrap(),
            (Some(Status::Down), vec![(5_000, Status::Up)])
        );
    }

    #[tokio::test]
    async fn the_problem_log_lists_warnings_outages_and_their_end() {
        let (_dir, pool, sensor) = setup().await;
        for (ts, a) in [
            (1_000, ok()),
            (2_000, fail()),
            (3_000, fail()),
            (5_000, ok()),
        ] {
            record_poll(&pool, sensor, ts, &[], &a).await.unwrap();
        }
        let log: Vec<_> = problem_log(&pool, None)
            .await
            .unwrap()
            .into_iter()
            .map(|e| (e.event.ts_ms, e.event.to, e.prev_ts_ms, e.next_ts_ms))
            .collect();
        // The first "up" is no recovery and stays out.
        assert_eq!(
            log,
            [
                (5_000, Status::Up, Some(3_000), None),
                (3_000, Status::Down, Some(2_000), Some(5_000)),
                (2_000, Status::Warning, Some(1_000), Some(3_000)),
            ]
        );
    }

    #[tokio::test]
    async fn only_the_newest_events_of_a_sensor_are_kept() {
        let (_dir, pool, sensor) = setup().await;
        // Every poll flips between up and warning, so each one logs an event.
        let polls = EVENTS_KEPT_PER_SENSOR + 20;
        for i in 0..polls {
            let a = if i % 2 == 0 { ok() } else { fail() };
            record_poll(&pool, sensor, (i + 1) * 1_000, &[], &a)
                .await
                .unwrap();
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, EVENTS_KEPT_PER_SENSOR);
        let oldest: i64 = sqlx::query_scalar("SELECT MIN(ts_ms) FROM events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(oldest, 21_000);
        // Before the oldest kept change, the history falls back to that change's "from".
        let (before, changes) = status_history(&pool, sensor, 20_500).await.unwrap();
        assert_eq!((before, changes.len()), (Some(Status::Warning), 1000));
        // Without any change in the window, the current status applies.
        let (before, changes) = status_history(&pool, sensor, polls * 1_000 + 1)
            .await
            .unwrap();
        assert_eq!((before, changes.len()), (Some(Status::Warning), 0));
    }

    #[tokio::test]
    async fn a_busy_sensor_does_not_push_out_the_changes_of_a_quiet_one() {
        let (_dir, pool, quiet) = setup().await;
        let busy = add_sensor(&pool, "Busy").await;
        record_poll(&pool, quiet, 500, &[], &ok()).await.unwrap();
        record_poll(&pool, quiet, 600, &[], &fail()).await.unwrap();
        for i in 0..EVENTS_KEPT_PER_SENSOR + 50 {
            let a = if i % 2 == 0 { ok() } else { fail() };
            record_poll(&pool, busy, (i + 1) * 1_000, &[], &a)
                .await
                .unwrap();
        }
        assert_eq!(
            event_totals(&pool, quiet).await.unwrap(),
            (2, Some(500)),
            "the quiet sensor keeps both of its changes"
        );
        assert_eq!(
            event_totals(&pool, busy).await.unwrap().0,
            EVENTS_KEPT_PER_SENSOR
        );
    }

    #[tokio::test]
    async fn events_come_in_pages_and_a_page_is_found_again_by_its_first_event() {
        let (_dir, pool, sensor) = setup().await;
        for i in 0..7 {
            let a = if i % 2 == 0 { ok() } else { fail() };
            record_poll(&pool, sensor, (i + 1) * 1_000, &[], &a)
                .await
                .unwrap();
        }
        let first = events_of_sensor(&pool, sensor, 3, 0).await.unwrap();
        let second = events_of_sensor(&pool, sensor, 3, 3).await.unwrap();
        let last = events_of_sensor(&pool, sensor, 3, 6).await.unwrap();
        let ts = |v: &[Event]| v.iter().map(|e| e.ts_ms).collect::<Vec<_>>();
        assert_eq!(ts(&first), [7_000, 6_000, 5_000]);
        assert_eq!(ts(&second), [4_000, 3_000, 2_000]);
        assert_eq!(ts(&last), [1_000]);
        assert!(
            events_of_sensor(&pool, sensor, 3, 7)
                .await
                .unwrap()
                .is_empty()
        );
        // Two more changes arrive: the page that began with `second[0]` now begins 2 events further down.
        record_poll(&pool, sensor, 8_000, &[], &fail())
            .await
            .unwrap();
        record_poll(&pool, sensor, 9_000, &[], &ok()).await.unwrap();
        assert_eq!(
            events_newer_than(&pool, sensor, second[0].id)
                .await
                .unwrap(),
            5
        );
        // An id that is gone (older than everything kept) counts everything as newer.
        assert_eq!(events_newer_than(&pool, sensor, 0).await.unwrap(), 9);
    }

    #[tokio::test]
    async fn the_problem_log_is_capped_and_can_be_limited_to_sensors() {
        let (_dir, pool, a) = setup().await;
        let b = add_sensor(&pool, "Other").await;
        for i in 0..(LOG_ENTRIES + 10) {
            let x = if i % 2 == 0 { ok() } else { fail() };
            record_poll(&pool, a, (i + 1) * 1_000, &[], &x)
                .await
                .unwrap();
        }
        record_poll(&pool, b, 1, &[], &ok()).await.unwrap();
        record_poll(&pool, b, 2, &[], &fail()).await.unwrap();
        assert_eq!(
            problem_log(&pool, None).await.unwrap().len(),
            usize::try_from(LOG_ENTRIES).unwrap()
        );
        // Sensor `b` is older than everything in the capped list, yet its own log is complete.
        let only_b = problem_log(&pool, Some(&[b])).await.unwrap();
        assert_eq!(only_b.len(), 1);
        assert_eq!(only_b[0].event.sensor_id, b);
        assert!(problem_log(&pool, Some(&[])).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn samples_and_state_are_written_together() {
        let (_dir, pool, sensor) = setup().await;
        let channel = crate::storage::sensors::get_sensor(&pool, sensor)
            .await
            .unwrap()
            .unwrap()
            .channels[0]
            .id;
        let samples = [Sample {
            channel_id: channel,
            ts_ms: 1_000,
            value: 42.0,
        }];
        record_poll(&pool, sensor, 1_000, &samples, &ok())
            .await
            .unwrap();
        let stored = crate::storage::sensors::samples_since(&pool, channel, 0)
            .await
            .unwrap();
        assert_eq!(stored, samples);
    }

    #[tokio::test]
    async fn list_states_and_cascade_on_sensor_delete() {
        let (_dir, pool, sensor) = setup().await;
        record_poll(&pool, sensor, 1_000, &[], &fail())
            .await
            .unwrap();
        let states = list_states(&pool).await.unwrap();
        assert_eq!(states[&sensor].status, Status::Warning);
        crate::storage::sensors::delete_sensor(&pool, sensor)
            .await
            .unwrap();
        assert!(list_states(&pool).await.unwrap().is_empty());
        assert!(recent_events(&pool, 10).await.unwrap().is_empty());
    }
}
