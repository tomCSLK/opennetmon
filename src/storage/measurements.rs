//! Reading measurements for charts and "latest value" displays.

use std::collections::BTreeMap;

use sqlx::SqlitePool;

use crate::error::Result;
use crate::sensors::Unit;
use crate::storage::sensors::ChannelRecord;

const HOUR_MS: i64 = 3_600_000;
/// Safety bound per channel; the buckets below keep real queries far under it.
const MAX_POINTS: i64 = 20_000;

/// Time range of a chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    Hour,
    Day,
    Week,
    Month,
}

impl Range {
    pub const ALL: [Range; 4] = [Range::Hour, Range::Day, Range::Week, Range::Month];

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == s)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hour => "1h",
            Self::Day => "24h",
            Self::Week => "7d",
            Self::Month => "30d",
        }
    }

    /// Catalog id of the label.
    pub fn label_id(self) -> &'static str {
        match self {
            Self::Hour => "range-hour",
            Self::Day => "range-day",
            Self::Week => "range-week",
            Self::Month => "range-month",
        }
    }

    /// Length of one aggregated point in seconds; `None` for raw samples (1 hour, 24 hours).
    /// Must match the bucketing in [`points`].
    pub fn step_secs(self) -> Option<i64> {
        match self {
            Self::Hour | Self::Day => None,
            Self::Week => Some(600),
            Self::Month => Some(3_600),
        }
    }

    fn duration_ms(self) -> i64 {
        match self {
            Self::Hour => HOUR_MS,
            Self::Day => 24 * HOUR_MS,
            Self::Week => 7 * 24 * HOUR_MS,
            Self::Month => 30 * 24 * HOUR_MS,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartSeries {
    pub key: String,
    pub unit: Unit,
    /// One value per entry of [`ChartData::ts`]; `None` where this channel has no sample.
    pub values: Vec<Option<f64>>,
}

/// Aligned series for uPlot: `ts` in seconds since the Unix epoch, ascending.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChartData {
    pub ts: Vec<i64>,
    pub series: Vec<ChartSeries>,
}

/// Points of one channel as `(ts_ms, value)`. Raw samples for up to 24 hours, 10-minute averages
/// for 7 days and the hourly rollup for 30 days (raw hourly averages if no rollup exists yet).
async fn points(
    pool: &SqlitePool,
    channel_id: i64,
    range: Range,
    now_ms: i64,
) -> Result<Vec<(i64, f64)>> {
    let from = now_ms - range.duration_ms();
    let raw = "SELECT ts, value FROM samples WHERE channel_id = ? AND ts >= ? AND ts <= ? ORDER BY ts LIMIT ?";
    let bucketed = "SELECT (ts / ?) * ?, AVG(value) FROM samples WHERE channel_id = ? AND ts >= ? AND ts <= ? \
                    GROUP BY ts / ? ORDER BY 1 LIMIT ?";
    let bucket = |size: i64| {
        sqlx::query_as::<_, (i64, f64)>(bucketed)
            .bind(size)
            .bind(size)
            .bind(channel_id)
            .bind(from)
            .bind(now_ms)
            .bind(size)
            .bind(MAX_POINTS)
    };
    Ok(match range {
        Range::Hour | Range::Day => {
            sqlx::query_as(raw)
                .bind(channel_id)
                .bind(from)
                .bind(now_ms)
                .bind(MAX_POINTS)
                .fetch_all(pool)
                .await?
        }
        Range::Week => bucket(600_000).fetch_all(pool).await?,
        Range::Month => {
            let hourly: Vec<(i64, f64)> = sqlx::query_as(
                "SELECT ts, avg FROM samples_1h WHERE channel_id = ? AND ts >= ? AND ts <= ? ORDER BY ts LIMIT ?",
            )
            .bind(channel_id)
            .bind(from - HOUR_MS)
            .bind(now_ms)
            .bind(MAX_POINTS)
            .fetch_all(pool)
            .await?;
            if hourly.is_empty() {
                bucket(HOUR_MS).fetch_all(pool).await?
            } else {
                hourly
            }
        }
    })
}

/// The chart data of all `channels` over `range`, ending at `now_ms`.
pub async fn chart_data(
    pool: &SqlitePool,
    channels: &[ChannelRecord],
    range: Range,
    now_ms: i64,
) -> Result<ChartData> {
    let mut rows: BTreeMap<i64, Vec<Option<f64>>> = BTreeMap::new();
    for (i, channel) in channels.iter().enumerate() {
        for (ts_ms, value) in points(pool, channel.id, range, now_ms).await? {
            let row = rows
                .entry(ts_ms.div_euclid(1000))
                .or_insert_with(|| vec![None; channels.len()]);
            row[i] = Some(value);
        }
    }
    let ts = rows.keys().copied().collect();
    let series = channels
        .iter()
        .enumerate()
        .map(|(i, c)| ChartSeries {
            key: c.key.clone(),
            unit: c.unit,
            values: rows.values().map(|row| row[i]).collect(),
        })
        .collect();
    Ok(ChartData { ts, series })
}

/// Averages of one channel in `count` equal buckets of `bucket_ms` starting at `from_ms`; `None`
/// for a bucket without samples. For the small charts of the sensor list.
pub async fn bucket_averages(
    pool: &SqlitePool,
    channel_id: i64,
    from_ms: i64,
    bucket_ms: i64,
    count: usize,
) -> Result<Vec<Option<f64>>> {
    let mut out = vec![None; count];
    if bucket_ms <= 0 {
        return Ok(out);
    }
    let end = from_ms + bucket_ms * i64::try_from(count).unwrap_or(0);
    let rows: Vec<(i64, f64)> = sqlx::query_as(
        "SELECT (ts - ?1) / ?2, AVG(value) FROM samples \
         WHERE channel_id = ?3 AND ts >= ?1 AND ts < ?4 GROUP BY 1",
    )
    .bind(from_ms)
    .bind(bucket_ms)
    .bind(channel_id)
    .bind(end)
    .fetch_all(pool)
    .await?;
    for (bucket, value) in rows {
        if let Some(slot) = usize::try_from(bucket).ok().and_then(|b| out.get_mut(b)) {
            *slot = Some(value);
        }
    }
    Ok(out)
}

/// The newest raw sample per channel: `channel_id -> (ts_ms, value)`.
pub async fn latest_values(
    pool: &SqlitePool,
    channels: &[ChannelRecord],
) -> Result<BTreeMap<i64, (i64, f64)>> {
    let mut out = BTreeMap::new();
    for channel in channels {
        let row: Option<(i64, f64)> = sqlx::query_as(
            "SELECT ts, value FROM samples WHERE channel_id = ? ORDER BY ts DESC LIMIT 1",
        )
        .bind(channel.id)
        .fetch_optional(pool)
        .await?;
        if let Some(row) = row {
            out.insert(channel.id, row);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maintenance::rollup;
    use crate::sensors::spec::SensorSpec;
    use crate::storage::sensors::{Sample, create_sensor, get_sensor, insert_samples};
    use crate::storage::{connect, migrate};

    async fn setup(spec: &SensorSpec) -> (tempfile::TempDir, SqlitePool, Vec<ChannelRecord>) {
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
        let sensor = create_sensor(&pool, device, "S", spec).await.unwrap();
        let channels = get_sensor(&pool, sensor).await.unwrap().unwrap().channels;
        (dir, pool, channels)
    }

    fn sample(channel_id: i64, ts_ms: i64, value: f64) -> Sample {
        Sample {
            channel_id,
            ts_ms,
            value,
        }
    }

    #[test]
    fn steps_match_the_point_resolution() {
        assert_eq!(Range::Hour.step_secs(), None);
        assert_eq!(Range::Day.step_secs(), None);
        assert_eq!(Range::Week.step_secs(), Some(600));
        assert_eq!(Range::Month.step_secs(), Some(3_600));
    }

    #[test]
    fn ranges_parse_from_their_names() {
        for r in Range::ALL {
            assert_eq!(Range::parse(r.as_str()), Some(r));
        }
        assert_eq!(Range::parse("1y"), None);
        assert_eq!(Range::parse(""), None);
    }

    #[tokio::test]
    async fn channels_are_aligned_on_a_shared_time_axis() {
        let (_dir, pool, ch) = setup(&SensorSpec::InterfaceTraffic { if_index: 1 }).await;
        let now = 10 * HOUR_MS;
        insert_samples(
            &pool,
            &[
                sample(ch[0].id, now - 30_000, 1.0),
                sample(ch[1].id, now - 30_000, 2.0),
                sample(ch[0].id, now - 20_000, 3.0), // only "in" has a value here
                sample(ch[0].id, now - 2 * HOUR_MS, 9.0), // outside the 1h range
            ],
        )
        .await
        .unwrap();
        let data = chart_data(&pool, &ch, Range::Hour, now).await.unwrap();
        assert_eq!(data.ts, [(now - 30_000) / 1000, (now - 20_000) / 1000]);
        assert_eq!(data.series[0].key, "in");
        assert_eq!(data.series[0].values, [Some(1.0), Some(3.0)]);
        assert_eq!(data.series[1].values, [Some(2.0), None]);
        // The wider range includes the older sample.
        let day = chart_data(&pool, &ch, Range::Day, now).await.unwrap();
        assert_eq!(day.ts.len(), 3);
    }

    #[tokio::test]
    async fn bucket_averages_leave_gaps_empty() {
        let (_dir, pool, ch) = setup(&SensorSpec::Ping).await;
        let from = 1_000_000;
        insert_samples(
            &pool,
            &[
                sample(ch[0].id, from - 1, 99.0),     // before the window
                sample(ch[0].id, from, 2.0),          // bucket 0
                sample(ch[0].id, from + 500, 4.0),    // bucket 0
                sample(ch[0].id, from + 2_100, 7.0),  // bucket 2
                sample(ch[0].id, from + 3_000, 99.0), // after the window
            ],
        )
        .await
        .unwrap();
        assert_eq!(
            bucket_averages(&pool, ch[0].id, from, 1_000, 3)
                .await
                .unwrap(),
            [Some(3.0), None, Some(7.0)]
        );
    }

    #[tokio::test]
    async fn a_week_is_averaged_into_ten_minute_buckets() {
        let (_dir, pool, ch) = setup(&SensorSpec::Uptime).await;
        let now = 100 * 24 * HOUR_MS;
        let bucket = now - 3 * 24 * HOUR_MS;
        let bucket = (bucket / 600_000) * 600_000;
        insert_samples(
            &pool,
            &[
                sample(ch[0].id, bucket + 1_000, 10.0),
                sample(ch[0].id, bucket + 60_000, 20.0),
                sample(ch[0].id, bucket + 600_000, 5.0),
            ],
        )
        .await
        .unwrap();
        let data = chart_data(&pool, &ch, Range::Week, now).await.unwrap();
        assert_eq!(data.ts, [bucket / 1000, (bucket + 600_000) / 1000]);
        assert_eq!(data.series[0].values, [Some(15.0), Some(5.0)]);
    }

    #[tokio::test]
    async fn a_month_uses_hourly_rollups_and_falls_back_to_raw_data() {
        let (_dir, pool, ch) = setup(&SensorSpec::Uptime).await;
        let now = 100 * 24 * HOUR_MS;
        let hour = ((now - 5 * 24 * HOUR_MS) / HOUR_MS) * HOUR_MS;
        insert_samples(
            &pool,
            &[
                sample(ch[0].id, hour + 1, 2.0),
                sample(ch[0].id, hour + 2, 4.0),
            ],
        )
        .await
        .unwrap();

        // No rollup yet: hourly averages straight from the raw data.
        let fallback = chart_data(&pool, &ch, Range::Month, now).await.unwrap();
        assert_eq!(
            (fallback.ts.clone(), fallback.series[0].values.clone()),
            (vec![hour / 1000], vec![Some(3.0)])
        );

        rollup(&pool, now).await.unwrap();
        sqlx::query("DELETE FROM samples")
            .execute(&pool)
            .await
            .unwrap();
        let rolled = chart_data(&pool, &ch, Range::Month, now).await.unwrap();
        assert_eq!(
            (rolled.ts, rolled.series[0].values.clone()),
            (vec![hour / 1000], vec![Some(3.0)])
        );
    }

    #[tokio::test]
    async fn no_data_gives_empty_series_and_latest_values_pick_the_newest() {
        let (_dir, pool, ch) = setup(&SensorSpec::InterfaceTraffic { if_index: 1 }).await;
        let empty = chart_data(&pool, &ch, Range::Day, 1_000_000).await.unwrap();
        assert!(empty.ts.is_empty());
        assert_eq!(empty.series.len(), 2);
        assert!(empty.series.iter().all(|s| s.values.is_empty()));
        assert!(latest_values(&pool, &ch).await.unwrap().is_empty());

        insert_samples(
            &pool,
            &[
                sample(ch[0].id, 1_000, 1.0),
                sample(ch[0].id, 5_000, 7.0),
                sample(ch[0].id, 3_000, 3.0),
            ],
        )
        .await
        .unwrap();
        let latest = latest_values(&pool, &ch).await.unwrap();
        assert_eq!(latest.get(&ch[0].id), Some(&(5_000, 7.0)));
        assert!(!latest.contains_key(&ch[1].id));
    }
}
