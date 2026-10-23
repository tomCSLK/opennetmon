//! Reading measurements for charts and "latest value" displays.

use std::collections::BTreeMap;

use sqlx::SqlitePool;

use crate::error::Result;
use crate::sensors::Unit;
use crate::storage::sensors::ChannelRecord;

const HOUR_MS: i64 = 3_600_000;
/// Safety bound per channel; the buckets below keep real queries far under it.
const MAX_POINTS: i64 = 20_000;
/// Points of an averaged chart (24 hours, 7 days, 30 days).
const CHART_POINTS: i64 = 120;

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

    /// Length of one aggregated point in seconds: the range split into [`CHART_POINTS`] buckets;
    /// `None` for raw samples (1 hour). Must match the bucketing in [`points`].
    pub fn step_secs(self) -> Option<i64> {
        match self {
            Self::Hour => None,
            _ => Some(self.duration_ms() / 1000 / CHART_POINTS),
        }
    }

    /// Start and length of the first of the [`CHART_POINTS`] buckets ending with the one that holds
    /// `now_ms`, both in ms; `None` for raw samples.
    fn buckets(self, now_ms: i64) -> Option<(i64, i64)> {
        let step = self.step_secs()? * 1000;
        Some(((now_ms.div_euclid(step) + 1 - CHART_POINTS) * step, step))
    }

    /// Minutes the table of values can reach back in this range (raw samples, newest first); the
    /// first is the default. Raw samples are kept 7 days by default, so 30 days stops there.
    pub fn table_spans(self) -> &'static [u32] {
        match self {
            Self::Hour => &[15, 30, 60],
            Self::Day => &[60, 360, 1440],
            Self::Week => &[360, 1440, 10_080],
            Self::Month => &[1440, 10_080],
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

/// Points of one channel as `(ts_ms, value)`. Raw samples for 1 hour; otherwise averages in
/// [`CHART_POINTS`] buckets of [`Range::step_secs`], aligned to multiples of the step so they stay
/// put between reloads (the newest bucket is still filling up). 24 hours and 7 days average the raw
/// samples, 30 days the hourly rollup (the raw samples if no rollup exists yet).
async fn points(
    pool: &SqlitePool,
    channel_id: i64,
    range: Range,
    now_ms: i64,
) -> Result<Vec<(i64, f64)>> {
    let Some((from, step)) = range.buckets(now_ms) else {
        return Ok(sqlx::query_as(
            "SELECT ts, value FROM samples WHERE channel_id = ? AND ts >= ? AND ts <= ? ORDER BY ts LIMIT ?",
        )
        .bind(channel_id)
        .bind(now_ms - range.duration_ms())
        .bind(now_ms)
        .bind(MAX_POINTS)
        .fetch_all(pool)
        .await?);
    };
    let bucketed = |table: &str, column: &str| {
        format!(
            "SELECT (ts / ?1) * ?1, AVG({column}) FROM {table} WHERE channel_id = ?2 AND ts >= ?3 AND ts <= ?4 \
             GROUP BY ts / ?1 ORDER BY 1 LIMIT ?5"
        )
    };
    let query = |sql: String| async move {
        sqlx::query_as::<_, (i64, f64)>(&sql)
            .bind(step)
            .bind(channel_id)
            .bind(from)
            .bind(now_ms)
            .bind(CHART_POINTS)
            .fetch_all(pool)
            .await
    };
    if range == Range::Month {
        let hourly = query(bucketed("samples_1h", "avg")).await?;
        if !hourly.is_empty() {
            return Ok(hourly);
        }
    }
    Ok(query(bucketed("samples", "value")).await?)
}

/// The chart data of all `channels` over `range`, ending at `now_ms`. Averaged ranges list every
/// bucket, empty ones as `None`, so the chart shows a gap instead of a line across it.
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
    if let (Some((from, step)), false) = (range.buckets(now_ms), rows.is_empty()) {
        for k in 0..CHART_POINTS {
            rows.entry((from + k * step).div_euclid(1000))
                .or_insert_with(|| vec![None; channels.len()]);
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

/// The raw samples of `channels` in the last `span_minutes` before `now_ms` (aligned like
/// [`chart_data`]; at most `limit`, the newest), and how many points the span holds in all.
/// For the table of values, which lists every poll whatever the chart averages.
pub async fn recent_samples(
    pool: &SqlitePool,
    channels: &[ChannelRecord],
    span_minutes: u32,
    now_ms: i64,
    limit: u32,
) -> Result<(ChartData, u64)> {
    let from = now_ms - i64::from(span_minutes) * 60_000;
    let mut rows: BTreeMap<i64, Vec<Option<f64>>> = BTreeMap::new();
    for (i, channel) in channels.iter().enumerate() {
        let points: Vec<(i64, f64)> = sqlx::query_as(
            "SELECT ts, value FROM samples WHERE channel_id = ? AND ts >= ? AND ts <= ? \
             ORDER BY ts DESC LIMIT ?",
        )
        .bind(channel.id)
        .bind(from)
        .bind(now_ms)
        .bind(limit)
        .fetch_all(pool)
        .await?;
        for (ts_ms, value) in points {
            let row = rows
                .entry(ts_ms.div_euclid(1000))
                .or_insert_with(|| vec![None; channels.len()]);
            row[i] = Some(value);
        }
    }
    // Channels polled together share their timestamps; should they differ, keep the newest rows.
    let skip = rows
        .len()
        .saturating_sub(usize::try_from(limit).unwrap_or(usize::MAX));
    let rows: Vec<(i64, Vec<Option<f64>>)> = rows.into_iter().skip(skip).collect();

    let mut total: i64 = 0;
    for channel in channels {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM samples WHERE channel_id = ? AND ts >= ? AND ts <= ?",
        )
        .bind(channel.id)
        .bind(from)
        .bind(now_ms)
        .fetch_one(pool)
        .await?;
        total = total.max(n);
    }

    let ts = rows.iter().map(|(ts, _)| *ts).collect();
    let series = channels
        .iter()
        .enumerate()
        .map(|(i, c)| ChartSeries {
            key: c.key.clone(),
            unit: c.unit,
            values: rows.iter().map(|(_, row)| row[i]).collect(),
        })
        .collect();
    Ok((ChartData { ts, series }, u64::try_from(total).unwrap_or(0)))
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

    /// `(ts, value)` of the buckets of the first series that have a value.
    fn filled(data: &ChartData) -> Vec<(i64, f64)> {
        data.ts
            .iter()
            .zip(&data.series[0].values)
            .filter_map(|(&ts, v)| v.map(|v| (ts, v)))
            .collect()
    }

    #[test]
    fn steps_match_the_point_resolution() {
        assert_eq!(Range::Hour.step_secs(), None);
        assert_eq!(Range::Day.step_secs(), Some(12 * 60));
        assert_eq!(Range::Week.step_secs(), Some(84 * 60));
        assert_eq!(Range::Month.step_secs(), Some(6 * 3_600));
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
        // The wider range includes the older sample, averaged with its neighbours per bucket.
        // Buckets of 12 minutes, the last one starting at `now`: the older sample lands in bucket 109,
        // the newer ones in bucket 118.
        let day = chart_data(&pool, &ch, Range::Day, now).await.unwrap();
        assert_eq!(day.ts.len(), 120);
        let filled: Vec<_> = (0..120)
            .filter(|&k| day.series[0].values[k].is_some())
            .map(|k| (k, day.series[0].values[k], day.series[1].values[k]))
            .collect();
        assert_eq!(
            filled,
            [(109, Some(9.0), None), (118, Some(2.0), Some(2.0))]
        );
    }

    #[tokio::test]
    async fn the_table_gets_the_newest_raw_samples_of_the_range() {
        let (_dir, pool, ch) = setup(&SensorSpec::InterfaceTraffic { if_index: 1 }).await;
        let now = 30 * 24 * HOUR_MS;
        let mut samples = Vec::new();
        // One poll a minute for three hours, both channels, plus one sample older than a day.
        for k in 0..180 {
            let ts = now - k * 60_000;
            samples.push(sample(ch[0].id, ts, k as f64));
            samples.push(sample(ch[1].id, ts, 1000.0 + k as f64));
        }
        samples.push(sample(ch[0].id, now - 25 * HOUR_MS, -1.0));
        insert_samples(&pool, &samples).await.unwrap();

        // Every poll of the span (61: both ends included), not averages.
        let (hour, total) = recent_samples(&pool, &ch, 60, now, 20_000).await.unwrap();
        assert_eq!((hour.ts.len(), total), (61, 61));
        assert_eq!(hour.ts.last(), Some(&(now / 1000)));
        assert_eq!(hour.ts[0], (now - HOUR_MS) / 1000);
        assert_eq!(hour.series[0].values[60], Some(0.0));
        assert_eq!(hour.series[1].values[0], Some(1060.0));
        // A day reaches all 180 polls; the limit keeps the newest.
        let (day, total) = recent_samples(&pool, &ch, 1440, now, 100).await.unwrap();
        assert_eq!((day.ts.len(), total), (100, 180));
        assert_eq!(day.ts[0], (now - 99 * 60_000) / 1000);
        // The week reaches the older sample.
        let (_, total) = recent_samples(&pool, &ch, 10_080, now, 20_000)
            .await
            .unwrap();
        assert_eq!(total, 181);
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
    async fn a_week_is_averaged_into_buckets_of_84_minutes() {
        let (_dir, pool, ch) = setup(&SensorSpec::Uptime).await;
        let step = 84 * 60_000;
        let now = 1_000 * step + 5_000;
        let bucket = now - 3 * 24 * HOUR_MS;
        let bucket = (bucket / step) * step;
        insert_samples(
            &pool,
            &[
                sample(ch[0].id, bucket + 1_000, 10.0),
                sample(ch[0].id, bucket + step - 1, 20.0),
                sample(ch[0].id, bucket + step, 5.0),
            ],
        )
        .await
        .unwrap();
        let data = chart_data(&pool, &ch, Range::Week, now).await.unwrap();
        assert_eq!(
            filled(&data),
            [(bucket / 1000, 15.0), ((bucket + step) / 1000, 5.0)]
        );
    }

    #[tokio::test]
    async fn averaged_ranges_show_120_points() {
        let (_dir, pool, ch) = setup(&SensorSpec::Ping).await;
        let now = 400 * 24 * HOUR_MS + 1_234_567;
        // One sample per minute over the last 31 days, so every range is covered with room to spare.
        let samples: Vec<Sample> = (0..31 * 24 * 60)
            .map(|i| sample(ch[0].id, now - i * 60_000, 1.0))
            .collect();
        insert_samples(&pool, &samples).await.unwrap();
        for range in [Range::Day, Range::Week, Range::Month] {
            let data = chart_data(&pool, &ch, range, now).await.unwrap();
            let step = range.step_secs().unwrap();
            assert_eq!(data.ts.len(), 120, "{range:?}");
            assert!(
                data.series[0].values.iter().all(Option::is_some),
                "{range:?}"
            );
            assert!(data.ts.windows(2).all(|w| w[1] - w[0] == step), "{range:?}");
            assert!(
                data.ts[119] * 1000 <= now && now < (data.ts[119] + step) * 1000,
                "{range:?}"
            );
        }
        let hour = chart_data(&pool, &ch, Range::Hour, now).await.unwrap();
        assert_eq!(hour.ts.len(), 61, "1 hour keeps the raw samples");
    }

    #[tokio::test]
    async fn a_month_uses_hourly_rollups_and_falls_back_to_raw_data() {
        let (_dir, pool, ch) = setup(&SensorSpec::Uptime).await;
        let now = 100 * 24 * HOUR_MS;
        let hour = ((now - 5 * 24 * HOUR_MS) / HOUR_MS) * HOUR_MS + 3 * HOUR_MS;
        let bucket = (hour / (6 * HOUR_MS)) * 6 * HOUR_MS;
        insert_samples(
            &pool,
            &[
                sample(ch[0].id, hour + 1, 2.0),
                sample(ch[0].id, hour + 2, 4.0),
            ],
        )
        .await
        .unwrap();

        // No rollup yet: 6-hour averages straight from the raw data.
        let fallback = chart_data(&pool, &ch, Range::Month, now).await.unwrap();
        assert_eq!(filled(&fallback), [(bucket / 1000, 3.0)]);

        rollup(&pool, now).await.unwrap();
        sqlx::query("DELETE FROM samples")
            .execute(&pool)
            .await
            .unwrap();
        let rolled = chart_data(&pool, &ch, Range::Month, now).await.unwrap();
        assert_eq!(filled(&rolled), [(bucket / 1000, 3.0)]);
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
