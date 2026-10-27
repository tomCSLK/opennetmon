//! Rollups and retention of measurements.
//!
//! Raw samples are aggregated into hourly and daily min/avg/max rows, the hourly availability into
//! daily sums; old data is deleted after the configured retention. Both steps are idempotent and cheap, so they simply run periodically.

use sqlx::SqlitePool;

use crate::error::Result;

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;
/// Hours before the newest rollup row that are recomputed (late samples, the unfinished hour).
const HOURLY_OVERLAP: i64 = 2 * HOUR_MS;
const DAILY_OVERLAP: i64 = DAY_MS;

/// How long data is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    pub raw_days: u32,
    pub hourly_days: u32,
    pub audit_days: u32,
}

/// What a maintenance run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    pub raw_deleted: u64,
    pub hourly_deleted: u64,
    pub events_deleted: u64,
    pub notifications_deleted: u64,
}

/// Rolls raw samples up into `samples_1h` and `samples_1d`.
///
/// Starts two hours (one day) before the newest existing rollup row, or at the oldest raw sample
/// if there are no rollups yet, and includes the current unfinished hour; later runs correct it.
pub async fn rollup(pool: &SqlitePool, now_ms: i64) -> Result<()> {
    let newest_hour: Option<i64> = sqlx::query_scalar("SELECT MAX(ts) FROM samples_1h")
        .fetch_one(pool)
        .await?;
    let from_hour = match newest_hour {
        Some(ts) => Some((ts - HOURLY_OVERLAP).max(0)),
        None => {
            sqlx::query_scalar("SELECT MIN(ts) FROM samples")
                .fetch_one(pool)
                .await?
        }
    };
    let Some(from_hour) = from_hour else {
        return Ok(()); // nothing measured yet
    };
    let from_hour = from_hour.div_euclid(HOUR_MS) * HOUR_MS;
    sqlx::query(
        "INSERT INTO samples_1h (channel_id, ts, min, avg, max, n) \
         SELECT channel_id, (ts / ?1) * ?1, MIN(value), AVG(value), MAX(value), COUNT(*) \
         FROM samples WHERE ts >= ?2 AND ts <= ?3 GROUP BY channel_id, (ts / ?1) \
         ON CONFLICT (channel_id, ts) DO UPDATE SET min = excluded.min, avg = excluded.avg, \
         max = excluded.max, n = excluded.n",
    )
    .bind(HOUR_MS)
    .bind(from_hour)
    .bind(now_ms)
    .execute(pool)
    .await?;

    let newest_day: Option<i64> = sqlx::query_scalar("SELECT MAX(ts) FROM samples_1d")
        .fetch_one(pool)
        .await?;
    let from_day = match newest_day {
        Some(ts) => (ts - DAILY_OVERLAP).max(0),
        None => from_hour,
    };
    let from_day = from_day.div_euclid(DAY_MS) * DAY_MS;
    sqlx::query(
        "INSERT INTO samples_1d (channel_id, ts, min, avg, max, n) \
         SELECT channel_id, (ts / ?1) * ?1, MIN(min), SUM(avg * n) / SUM(n), MAX(max), SUM(n) \
         FROM samples_1h WHERE ts >= ?2 GROUP BY channel_id, (ts / ?1) \
         ON CONFLICT (channel_id, ts) DO UPDATE SET min = excluded.min, avg = excluded.avg, \
         max = excluded.max, n = excluded.n",
    )
    .bind(DAY_MS)
    .bind(from_day)
    .execute(pool)
    .await?;
    crate::storage::availability::rollup(pool).await
}

/// Deletes raw samples (with the failed polls), hourly rollups and events older than their
/// retention. Daily rollups stay.
pub async fn apply_retention(
    pool: &SqlitePool,
    now_ms: i64,
    retention: Retention,
) -> Result<Report> {
    let raw_before = now_ms - i64::from(retention.raw_days) * DAY_MS;
    let hourly_before = now_ms - i64::from(retention.hourly_days) * DAY_MS;
    let raw_deleted = sqlx::query("DELETE FROM samples WHERE ts < ?")
        .bind(raw_before)
        .execute(pool)
        .await?
        .rows_affected();
    sqlx::query("DELETE FROM poll_problems WHERE ts < ?")
        .bind(raw_before)
        .execute(pool)
        .await?;
    let hourly_deleted = sqlx::query("DELETE FROM samples_1h WHERE ts < ?")
        .bind(hourly_before)
        .execute(pool)
        .await?
        .rows_affected();
    let events_deleted = sqlx::query("DELETE FROM events WHERE ts_ms < ?")
        .bind(hourly_before)
        .execute(pool)
        .await?
        .rows_affected();
    let notifications_deleted = sqlx::query("DELETE FROM notifications WHERE ts_ms < ?")
        .bind(hourly_before)
        .execute(pool)
        .await?
        .rows_affected();
    crate::storage::availability::apply_retention(pool, hourly_before).await?;
    let audit_before = now_ms - i64::from(retention.audit_days) * DAY_MS;
    crate::storage::audit::apply_retention(pool, audit_before).await?;
    Ok(Report {
        raw_deleted,
        hourly_deleted,
        events_deleted,
        notifications_deleted,
    })
}

/// One maintenance run: rollups first, so nothing is deleted before it was aggregated.
pub async fn run(pool: &SqlitePool, now_ms: i64, retention: Retention) -> Result<Report> {
    rollup(pool, now_ms).await?;
    apply_retention(pool, now_ms, retention).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensors::spec::SensorSpec;
    use crate::storage::sensors::{Sample, create_sensor, get_sensor, insert_samples};
    use crate::storage::{connect, migrate};

    /// A migrated database with one sensor; returns its first channel id.
    async fn setup() -> (tempfile::TempDir, SqlitePool, i64) {
        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("m.db")).await.unwrap();
        migrate(&pool).await.unwrap();
        sqlx::query("INSERT INTO credential_profiles (name, version, community_enc) VALUES ('p', 'v2c', x'01')")
            .execute(&pool)
            .await
            .unwrap();
        let device: i64 = sqlx::query_scalar(
            "INSERT INTO devices (name, host, profile_id) VALUES ('d', '192.0.2.1', 1) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let sensor = create_sensor(&pool, device, "Uptime", &SensorSpec::Uptime)
            .await
            .unwrap();
        let channel = get_sensor(&pool, sensor).await.unwrap().unwrap().channels[0].id;
        (dir, pool, channel)
    }

    async fn add(pool: &SqlitePool, channel_id: i64, points: &[(i64, f64)]) {
        let samples: Vec<_> = points
            .iter()
            .map(|(ts_ms, value)| Sample {
                channel_id,
                ts_ms: *ts_ms,
                value: *value,
            })
            .collect();
        insert_samples(pool, &samples).await.unwrap();
    }

    async fn hourly(pool: &SqlitePool) -> Vec<(i64, f64, f64, f64, i64)> {
        sqlx::query_as("SELECT ts, min, avg, max, n FROM samples_1h ORDER BY ts")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn samples_are_aggregated_per_hour() {
        let (_dir, pool, ch) = setup().await;
        let base = 10 * HOUR_MS;
        add(
            &pool,
            ch,
            &[
                (base, 1.0),
                (base + 60_000, 3.0),
                (base + HOUR_MS, 10.0),
                (base + HOUR_MS + 1, 20.0),
                (base + HOUR_MS + 2, 30.0),
            ],
        )
        .await;
        rollup(&pool, base + 2 * HOUR_MS).await.unwrap();
        assert_eq!(
            hourly(&pool).await,
            [
                (base, 1.0, 2.0, 3.0, 2),
                (base + HOUR_MS, 10.0, 20.0, 30.0, 3)
            ]
        );
    }

    #[tokio::test]
    async fn rollups_are_idempotent_and_pick_up_new_samples() {
        let (_dir, pool, ch) = setup().await;
        let base = 5 * HOUR_MS;
        add(&pool, ch, &[(base, 2.0)]).await;
        rollup(&pool, base + 10).await.unwrap();
        rollup(&pool, base + 10).await.unwrap();
        assert_eq!(hourly(&pool).await, [(base, 2.0, 2.0, 2.0, 1)]);
        // A later sample in the same (still open) hour corrects the row.
        add(&pool, ch, &[(base + 1_000, 4.0)]).await;
        rollup(&pool, base + 2_000).await.unwrap();
        assert_eq!(hourly(&pool).await, [(base, 2.0, 3.0, 4.0, 2)]);
    }

    #[tokio::test]
    async fn daily_rollups_weight_hours_by_their_sample_count() {
        let (_dir, pool, ch) = setup().await;
        let day = 3 * DAY_MS;
        // Hour 0: one sample of 10. Hour 1: three samples of 20 -> weighted average 17.5.
        add(
            &pool,
            ch,
            &[
                (day, 10.0),
                (day + HOUR_MS, 20.0),
                (day + HOUR_MS + 1, 20.0),
                (day + HOUR_MS + 2, 20.0),
            ],
        )
        .await;
        rollup(&pool, day + 2 * HOUR_MS).await.unwrap();
        let daily: Vec<(i64, f64, f64, f64, i64)> =
            sqlx::query_as("SELECT ts, min, avg, max, n FROM samples_1d")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(daily, [(day, 10.0, 17.5, 20.0, 4)]);
    }

    #[tokio::test]
    async fn an_empty_database_rolls_up_nothing() {
        let (_dir, pool, _) = setup().await;
        rollup(&pool, 1_000).await.unwrap();
        assert!(hourly(&pool).await.is_empty());
    }

    #[tokio::test]
    async fn retention_deletes_old_raw_hourly_and_events_but_keeps_daily() {
        let (_dir, pool, ch) = setup().await;
        let now = 500 * DAY_MS;
        let old_raw = now - 8 * DAY_MS;
        let new_raw = now - 6 * DAY_MS;
        let very_old = now - 400 * DAY_MS;
        add(
            &pool,
            ch,
            &[(old_raw, 1.0), (new_raw, 2.0), (very_old, 3.0)],
        )
        .await;
        rollup(&pool, now).await.unwrap();
        sqlx::query("INSERT INTO events (ts_ms, sensor_id, to_status, message) VALUES (?, 1, 'up', 'old'), (?, 1, 'up', 'new')")
            .bind(very_old)
            .bind(now - DAY_MS)
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query("INSERT INTO notifications (ts_ms, kind, subject, recipients, ok) VALUES (?, 'alert', 's', 'r', 1), (?, 'alert', 's', 'r', 1)")
            .bind(very_old)
            .bind(now - DAY_MS)
            .execute(&pool)
            .await
            .unwrap();

        let report = apply_retention(
            &pool,
            now,
            Retention {
                raw_days: 7,
                hourly_days: 365,
                audit_days: 365,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            report,
            Report {
                raw_deleted: 2,
                hourly_deleted: 1,
                events_deleted: 1,
                notifications_deleted: 1
            }
        );
        let raw: Vec<(i64,)> = sqlx::query_as("SELECT ts FROM samples")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(raw, [(new_raw,)]);
        let daily: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM samples_1d")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(daily, 3, "daily rollups are kept forever");
    }

    #[tokio::test]
    async fn run_aggregates_before_it_deletes() {
        let (_dir, pool, ch) = setup().await;
        let now = 100 * DAY_MS;
        add(&pool, ch, &[(now - 10 * DAY_MS, 5.0)]).await;
        run(
            &pool,
            now,
            Retention {
                raw_days: 7,
                hourly_days: 365,
                audit_days: 365,
            },
        )
        .await
        .unwrap();
        let raw: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM samples")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(raw, 0);
        assert_eq!(
            hourly(&pool).await.len(),
            1,
            "the deleted raw sample lives on in the rollup"
        );
    }
}
