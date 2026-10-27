//! Persistence of availability (see [`crate::availability`]): `availability_1h`, written by every
//! poll, and the daily sums `availability_1d`.

use std::collections::HashMap;

use sqlx::{SqliteConnection, SqlitePool};

use crate::availability::{DAY_MS, HOUR_MS, Totals, counted_span, split_hours};
use crate::error::Result;
use crate::status::Status;

fn column(status: Status) -> &'static str {
    match status {
        Status::Up => "up_ms",
        Status::Warning => "warning_ms",
        Status::Down => "down_ms",
        Status::Unknown => "unknown_ms",
        Status::Paused => "paused_ms",
    }
}

/// Adds the time since the poll before (`last_poll_ms`, in `status`) to the hours it covers, unless
/// it is a gap (see [`counted_span`]). Part of the transaction of `storage::status::record_poll`.
pub(crate) async fn count(
    conn: &mut SqliteConnection,
    sensor_id: i64,
    status: Status,
    last_poll_ms: i64,
    now_ms: i64,
) -> Result<()> {
    let interval: Option<i64> = sqlx::query_scalar(
        "SELECT COALESCE(s.interval_secs, d.interval_secs) FROM sensors s \
         JOIN devices d ON d.id = s.device_id WHERE s.id = ?",
    )
    .bind(sensor_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(interval) = interval.and_then(|i| u32::try_from(i).ok()) else {
        return Ok(());
    };
    let Some((from, to)) = counted_span(last_poll_ms, now_ms, interval) else {
        return Ok(());
    };
    let col = column(status);
    for (hour, ms) in split_hours(from, to) {
        sqlx::query(&format!(
            "INSERT INTO availability_1h (sensor_id, ts, {col}) VALUES (?, ?, ?) \
             ON CONFLICT (sensor_id, ts) DO UPDATE SET {col} = {col} + excluded.{col}"
        ))
        .bind(sensor_id)
        .bind(hour)
        .bind(ms)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Sums the hours into `availability_1d`, from a day before the newest daily row (the unfinished
/// day included; later runs correct it).
pub async fn rollup(pool: &SqlitePool) -> Result<()> {
    let newest: Option<i64> = sqlx::query_scalar("SELECT MAX(ts) FROM availability_1d")
        .fetch_one(pool)
        .await?;
    let from = newest.map_or(0, |ts| (ts - DAY_MS).max(0));
    sqlx::query(
        "INSERT INTO availability_1d (sensor_id, ts, up_ms, warning_ms, down_ms, unknown_ms, paused_ms) \
         SELECT sensor_id, (ts / ?1) * ?1, SUM(up_ms), SUM(warning_ms), SUM(down_ms), SUM(unknown_ms), SUM(paused_ms) \
         FROM availability_1h WHERE ts >= ?2 GROUP BY sensor_id, (ts / ?1) \
         ON CONFLICT (sensor_id, ts) DO UPDATE SET up_ms = excluded.up_ms, warning_ms = excluded.warning_ms, \
         down_ms = excluded.down_ms, unknown_ms = excluded.unknown_ms, paused_ms = excluded.paused_ms",
    )
    .bind(DAY_MS)
    .bind(from)
    .execute(pool)
    .await?;
    Ok(())
}

/// Deletes the hours before `before_ms` (after [`rollup`]).
pub async fn apply_retention(pool: &SqlitePool, before_ms: i64) -> Result<u64> {
    Ok(sqlx::query("DELETE FROM availability_1h WHERE ts < ?")
        .bind(before_ms)
        .execute(pool)
        .await?
        .rows_affected())
}

type TotalsRow = (i64, i64, i64, i64, i64, i64);

/// The totals per sensor for `from_ms..to_ms`, to the hour: hours that start in the range, and for
/// days whose hours were deleted already their daily sums. `sensor` limits it to one sensor.
pub async fn totals(
    pool: &SqlitePool,
    from_ms: i64,
    to_ms: i64,
    sensor: Option<i64>,
) -> Result<HashMap<i64, Totals>> {
    let from = from_ms.div_euclid(HOUR_MS) * HOUR_MS;
    let rows: Vec<TotalsRow> = sqlx::query_as(
        "WITH cut AS (SELECT COALESCE(MIN(ts), ?2) AS ts FROM availability_1h), \
         parts AS ( \
             SELECT sensor_id, up_ms, warning_ms, down_ms, unknown_ms, paused_ms FROM availability_1h \
             WHERE ts >= ?1 AND ts < ?2 \
             UNION ALL \
             SELECT sensor_id, up_ms, warning_ms, down_ms, unknown_ms, paused_ms FROM availability_1d \
             WHERE ts >= ?1 AND ts < ?2 AND ts + ?3 <= (SELECT ts FROM cut) \
         ) \
         SELECT sensor_id, SUM(up_ms), SUM(warning_ms), SUM(down_ms), SUM(unknown_ms), SUM(paused_ms) \
         FROM parts WHERE ?4 IS NULL OR sensor_id = ?4 GROUP BY sensor_id",
    )
    .bind(from)
    .bind(to_ms)
    .bind(DAY_MS)
    .bind(sensor)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, up_ms, warning_ms, down_ms, unknown_ms, paused_ms)| {
            (
                id,
                Totals {
                    up_ms,
                    warning_ms,
                    down_ms,
                    unknown_ms,
                    paused_ms,
                },
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{DeviceInput, Polling};
    use crate::sensors::spec::SensorSpec;
    use crate::storage::{connect, devices::save_device, migrate, sensors::create_sensor};

    async fn setup() -> (tempfile::TempDir, SqlitePool, i64) {
        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("a.db")).await.unwrap();
        migrate(&pool).await.unwrap();
        let device = save_device(
            &pool,
            None,
            DeviceInput {
                kind: Default::default(),
                name: "d".to_owned(),
                host: "192.0.2.1".to_owned(),
                group_id: None,
                profile_id: None,
                polling: Polling::default(),
            },
        )
        .await
        .unwrap();
        let sensor = create_sensor(&pool, device, "Ping", &SensorSpec::Ping)
            .await
            .unwrap();
        (dir, pool, sensor)
    }

    async fn add(pool: &SqlitePool, sensor: i64, status: Status, from: i64, to: i64) {
        let mut conn = pool.acquire().await.unwrap();
        count(&mut conn, sensor, status, from, to).await.unwrap();
    }

    #[tokio::test]
    async fn polls_add_their_time_per_hour_and_roll_up_per_day() {
        let (_dir, pool, s) = setup().await;
        // Default interval 60 s: spans up to 180 s count.
        let t0 = 10 * DAY_MS + HOUR_MS - 30_000;
        add(&pool, s, Status::Up, t0, t0 + 60_000).await;
        add(&pool, s, Status::Down, t0 + 60_000, t0 + 120_000).await;
        add(&pool, s, Status::Up, t0 + 120_000, t0 + 900_000).await; // a gap
        add(&pool, s, Status::Paused, t0 + 120_000, t0 + 150_000).await;
        let all = totals(&pool, 0, 20 * DAY_MS, None).await.unwrap();
        let t = all[&s];
        assert_eq!((t.up_ms, t.down_ms, t.paused_ms), (60_000, 60_000, 30_000));
        assert_eq!(t.percent(), Some(50.0));
        // Only the second hour.
        let second = totals(&pool, 10 * DAY_MS + HOUR_MS, 20 * DAY_MS, Some(s))
            .await
            .unwrap();
        assert_eq!(second[&s].up_ms, 30_000);
        assert_eq!(second[&s].down_ms, 60_000);

        rollup(&pool).await.unwrap();
        rollup(&pool).await.unwrap();
        // Once the hours are deleted, the day still counts.
        assert_eq!(apply_retention(&pool, 15 * DAY_MS).await.unwrap(), 2);
        let all = totals(&pool, 0, 20 * DAY_MS, None).await.unwrap();
        assert_eq!(all[&s], t);
        assert!(
            totals(&pool, 11 * DAY_MS, 20 * DAY_MS, None)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
