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

/// Status changes kept in `events`; older ones are deleted whenever a new one is written.
pub const EVENTS_KEPT: i64 = 500;

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
    let next = advance(prev.as_ref(), assessment, ts_ms);
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
        sqlx::query(
            "DELETE FROM events WHERE id <= (SELECT id FROM events ORDER BY id DESC LIMIT 1 OFFSET ?)",
        )
        .bind(EVENTS_KEPT)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(change)
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

/// The newest events of one sensor first.
pub async fn events_of_sensor(pool: &SqlitePool, sensor_id: i64, limit: u32) -> Result<Vec<Event>> {
    sqlx::query_as::<_, EventRow>(&format!(
        "{EVENT_SELECT} WHERE sensor_id = ? ORDER BY ts_ms DESC, id DESC LIMIT ?"
    ))
    .bind(sensor_id)
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(event_from_row)
    .collect()
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

/// Warnings, outages and their ends, newest first.
pub async fn problem_log(pool: &SqlitePool) -> Result<Vec<LogEntry>> {
    let rows: Vec<LogRow> = sqlx::query_as(
        "SELECT id, ts_ms, sensor_id, from_status, to_status, message, prev_ts, next_ts FROM ( \
               SELECT *, LAG(ts_ms) OVER w AS prev_ts, LEAD(ts_ms) OVER w AS next_ts FROM events \
               WINDOW w AS (PARTITION BY sensor_id ORDER BY ts_ms, id)) \
             WHERE to_status IN ('warning', 'down') \
                OR (to_status = 'up' AND from_status IN ('warning', 'down')) \
             ORDER BY ts_ms DESC, id DESC",
    )
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

    fn ok() -> Assessment {
        Assessment::Healthy("OK".into())
    }

    fn fail() -> Assessment {
        Assessment::Failed("timeout".into())
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
        let log: Vec<_> = problem_log(&pool)
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
    async fn only_the_newest_events_are_kept() {
        let (_dir, pool, sensor) = setup().await;
        // Every poll flips between up and warning, so each one logs an event.
        let polls = EVENTS_KEPT + 20;
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
        assert_eq!(count, EVENTS_KEPT);
        let oldest: i64 = sqlx::query_scalar("SELECT MIN(ts_ms) FROM events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(oldest, 21_000);
        // Before the oldest kept change, the history falls back to that change's "from".
        let (before, changes) = status_history(&pool, sensor, 20_500).await.unwrap();
        assert_eq!((before, changes.len()), (Some(Status::Warning), 500));
        // Without any change in the window, the current status applies.
        let (before, changes) = status_history(&pool, sensor, polls * 1_000 + 1)
            .await
            .unwrap();
        assert_eq!((before, changes.len()), (Some(Status::Warning), 0));
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
