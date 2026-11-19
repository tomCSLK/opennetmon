//! What an alert mail tells beyond the change: group and address, the latest value and the limits,
//! availability and outages of the last 7 days, the history of the last 6 hours and what depends on
//! the sensor. Read once per mail with a few small queries per change; anything that fails is left
//! out with a warning in the log, so the mail always goes out.

use std::collections::{BTreeMap, HashMap};

use sqlx::SqlitePool;

use crate::devices::{Device, Group};
use crate::error::Result;
use crate::i18n::Lang;
use crate::mail::view::{self, LegendItem, Tone};
use crate::pause::Pauses;
use crate::sensors::Unit;
use crate::sensors::format::channel_label;
use crate::sensors::local::LocalMetric;
use crate::sensors::spec::SensorSpec;
use crate::status::{Breach, Status, Thresholds};
use crate::storage::sensors::{ChannelRecord, SensorRecord};
use crate::web::sensor_text::ValueFormat;

use super::AlertEvent;
use super::compose::{
    CHART_HEIGHT, Dependent, Details, History, MINI_HEIGHT, MINI_ROWS, Other, order,
};

const HOUR_MS: i64 = 3_600_000;
/// The history shown: the last 6 hours.
pub const HISTORY_MS: i64 = 6 * HOUR_MS;
/// Availability and outages: the last 7 days.
const WEEK_MS: i64 = 7 * 24 * HOUR_MS;
/// Shortest stretch of a bar in the history of a single change (72 bars), and in the small ones of a
/// batch (24 bars); slow sensors get longer ones, at least two polls each.
const BUCKET_MS: i64 = 5 * 60_000;
const MINI_BUCKET_MS: i64 = 15 * 60_000;
/// The trend of a warning: the last 2 hours.
const TREND_MS: i64 = 2 * HOUR_MS;
/// Other sensors of the device a warning lists.
const MAX_OTHERS: usize = 5;
/// Polling interval if the device is unknown.
const DEFAULT_INTERVAL_SECS: u32 = 60;

/// Logs a failed read and goes on without it.
fn soft<T>(result: Result<T>, what: &str, sensor_id: i64) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(e) => {
            tracing::warn!(sensor_id, error = %e, "alert mail goes out without the {what}");
            None
        }
    }
}

/// The names of the groups above a device, from the top.
fn group_path(groups: &[Group], group_id: Option<i64>) -> Vec<String> {
    let mut path = Vec::new();
    let mut next = group_id;
    while let Some(id) = next {
        // Bounded by the number of groups, so a broken parent chain cannot loop.
        if path.len() > groups.len() {
            break;
        }
        let Some(group) = groups.iter().find(|g| g.id == id) else {
            break;
        };
        path.push(group.name.clone());
        next = group.parent_id;
    }
    path.reverse();
    path
}

/// Whether the sensor's value is a status (shown as coloured columns instead of bars).
fn status_valued(sensor: &SensorRecord) -> bool {
    matches!(
        sensor.spec,
        SensorSpec::InterfaceStatus { .. }
            | SensorSpec::Uptime
            | SensorSpec::Local(LocalMetric::Uptime)
    ) || sensor.channels.iter().any(|c| c.unit == Unit::State)
}

/// The channel a mail is about: one whose latest value is beyond a limit, else the first one the
/// lists show.
fn main_channel<'a>(
    sensor: &'a SensorRecord,
    latest: &BTreeMap<i64, (i64, f64)>,
) -> Option<&'a ChannelRecord> {
    sensor
        .channels
        .iter()
        .find(|c| {
            latest
                .get(&c.id)
                .is_some_and(|(_, v)| c.thresholds.breach(*v).is_some())
        })
        .or_else(|| {
            sensor
                .channels
                .iter()
                .find(|c| sensor.spec.in_lists(&c.key))
        })
        .or_else(|| sensor.channels.first())
}

/// How values of a channel are written: the unit of a custom OID, the labels of rated states.
fn value_format(sensor: &SensorRecord, channel: &ChannelRecord) -> ValueFormat {
    ValueFormat {
        unit_label: match &sensor.spec {
            SensorSpec::Oid { unit, .. } => unit.clone(),
            _ => None,
        },
        labels: channel
            .states
            .iter()
            .map(|s| (s.value, s.label.clone()))
            .collect(),
    }
}

/// The limits of a channel, each with its tone: "Warning above 75 °C".
fn limits(
    t: Lang,
    format: &ValueFormat,
    unit: Unit,
    thresholds: &Thresholds,
) -> Vec<(Tone, String)> {
    [
        (
            Tone::Warning,
            "mail-limit-warn-above",
            thresholds.warn_above,
        ),
        (
            Tone::Warning,
            "mail-limit-warn-below",
            thresholds.warn_below,
        ),
        (Tone::Down, "mail-limit-down-above", thresholds.down_above),
        (Tone::Down, "mail-limit-down-below", thresholds.down_below),
    ]
    .into_iter()
    .filter_map(|(tone, id, value)| {
        value.map(|v| (tone, t.get1(id, "value", format.format(t, unit, v))))
    })
    .collect()
}

/// Bucket length and count for the history: at least `min_ms` and two polls per bucket.
fn buckets(min_ms: i64, interval_secs: u32) -> (i64, usize) {
    let bucket = min_ms.max(i64::from(interval_secs) * 2_000);
    let count = (HISTORY_MS + bucket - 1) / bucket;
    (bucket, usize::try_from(count).unwrap_or(0))
}

/// A drawn history and the bucket averages behind it (empty for status columns).
struct Drawn {
    history: History,
    values: Vec<Option<f64>>,
    bucket_ms: i64,
}

/// Unit and divisor of the chart axis: bit rates in kbit/s, Mbit/s or Gbit/s by their largest value,
/// data volumes in GB, others as stored.
fn chart_unit(format: &ValueFormat, unit: Unit, max: f64) -> (String, f64) {
    if let Some(label) = &format.unit_label {
        let label = if unit == Unit::PerSecond {
            format!("{label}/s")
        } else {
            label.clone()
        };
        return (label, 1.0);
    }
    let (label, factor) = match unit {
        Unit::BitsPerSecond if max >= 1e9 => ("Gbit/s", 1e9),
        Unit::BitsPerSecond if max >= 1e6 => ("Mbit/s", 1e6),
        Unit::BitsPerSecond if max >= 1e3 => ("kbit/s", 1e3),
        Unit::BitsPerSecond => ("bit/s", 1.0),
        other => (
            crate::sensors::format::input_unit(other),
            crate::sensors::format::input_factor(other),
        ),
    };
    (label.to_owned(), factor)
}

/// The history as a picture ([`crate::chart_png`]): the raw samples of the main channel, its limits,
/// the outages of the window. `None` (with a warning in the log) if there are no values or the
/// picture fails; the mail then shows the bars.
#[allow(clippy::too_many_arguments)]
async fn picture(
    pool: &SqlitePool,
    t: Lang,
    sensor: &SensorRecord,
    ch: &ChannelRecord,
    periods: &[(i64, i64, Status)],
    (from_ms, now_ms): (i64, i64),
    interval_secs: u32,
    to: Status,
) -> Option<Vec<u8>> {
    use crate::chart_png::{Band, ChartSpec, Colour, Threshold, render_png};
    let samples = soft(
        crate::storage::sensors::samples_since(pool, ch.id, from_ms).await,
        "chart samples",
        sensor.id,
    )?;
    let values: Vec<(i64, f64)> = samples
        .iter()
        .filter(|s| s.ts_ms <= now_ms && s.value.is_finite())
        .map(|s| (s.ts_ms, s.value))
        .collect();
    if values.is_empty() {
        tracing::warn!(
            sensor_id = sensor.id,
            "alert mail shows bars: no values for the chart"
        );
        return None;
    }
    let format = value_format(sensor, ch);
    let max = values.iter().map(|(_, v)| v.abs()).fold(0.0, f64::max);
    let (unit, factor) = chart_unit(&format, ch.unit, max);
    let label = |id: &str, v: f64| t.get1(id, "value", format.format(t, ch.unit, v));
    let lim = &ch.thresholds;
    let thresholds = [
        (lim.warn_above, Band::Warning, "mail-chart-warn-above"),
        (lim.warn_below, Band::Warning, "mail-chart-warn-below"),
        (lim.down_above, Band::Down, "mail-chart-down-above"),
        (lim.down_below, Band::Down, "mail-chart-down-below"),
    ]
    .into_iter()
    .filter_map(|(value, band, id)| {
        value.map(|v| Threshold {
            value: v / factor,
            band,
            label: label(id, v),
        })
    })
    .collect();
    let outages: Vec<(i64, i64)> = periods
        .iter()
        .filter(|(_, _, status)| *status == Status::Down)
        .map(|(a, b, _)| (*a, *b))
        .collect();
    let outage_label = outages.last().map(|(a, b)| match to {
        Status::Up => view::duration(t, b - a),
        Status::Down if answers(&sensor.spec) => t.get("mail-chart-no-answer"),
        _ => t.get("mail-chart-outage"),
    });
    let temperature = ch.unit == Unit::Celsius || unit.contains('°');
    let spec = ChartSpec {
        start_ms: from_ms,
        end_ms: now_ms,
        points: values
            .into_iter()
            .map(|(ts, v)| (ts, Some(v / factor)))
            .collect(),
        unit,
        thresholds,
        outages,
        outage_label,
        colour: if temperature {
            Colour::Secondary
        } else {
            Colour::Primary
        },
        time_zone: crate::schedule::time_zone().clone(),
        max_gap_ms: Some(i64::from(interval_secs) * 2_500),
        decimal_comma: t == Lang::De,
    };
    match tokio::task::spawn_blocking(move || render_png(&spec)).await {
        Ok(Ok(png)) => Some(png),
        Ok(Err(e)) => {
            tracing::warn!(sensor_id = sensor.id, error = %e, "alert mail shows bars: chart failed");
            None
        }
        Err(e) => {
            tracing::warn!(sensor_id = sensor.id, error = %e, "alert mail shows bars: chart task failed");
            None
        }
    }
}

/// The history of the last 6 hours: bars of the channel's bucket averages with the outages
/// tinted, or coloured status columns for sensors whose value is a status. With `picture_for` (the
/// status the change went to) a value channel also gets the picture.
#[allow(clippy::too_many_arguments)]
async fn history(
    pool: &SqlitePool,
    t: Lang,
    sensor: &SensorRecord,
    channel: Option<&ChannelRecord>,
    interval_secs: u32,
    now_ms: i64,
    mini: bool,
    picture_for: Option<Status>,
) -> Result<Drawn> {
    let (min_ms, height) = if mini {
        (MINI_BUCKET_MS, MINI_HEIGHT)
    } else {
        (BUCKET_MS, CHART_HEIGHT)
    };
    let (bucket_ms, count) = buckets(min_ms, interval_secs);
    let from_ms = now_ms - bucket_ms * i64::try_from(count).unwrap_or(0);
    let (start, changes) = crate::storage::status::status_history(pool, sensor.id, from_ms).await?;
    let periods = crate::web::sparkline::periods(start, &changes, from_ms, now_ms);
    let statuses = view::bucket_status(&periods, from_ms, bucket_ms, count);
    let channel = channel.filter(|_| !status_valued(sensor));
    let mut png = None;
    let (title_channel, bars, legend, values) = match channel {
        None => (
            t.get("channel-oper-status"),
            view::status_bars(&statuses, height),
            Vec::new(),
            Vec::new(),
        ),
        Some(ch) => {
            let values = crate::storage::measurements::bucket_averages(
                pool, ch.id, from_ms, bucket_ms, count,
            )
            .await?;
            if let Some(to) = picture_for {
                png = picture(
                    pool,
                    t,
                    sensor,
                    ch,
                    &periods,
                    (from_ms, now_ms),
                    interval_secs,
                    to,
                )
                .await;
            }
            let format = value_format(sensor, ch);
            let mut legend: Vec<LegendItem> = limits(t, &format, ch.unit, &ch.thresholds)
                .into_iter()
                .map(|(tone, text)| LegendItem {
                    text,
                    tone: Some(tone),
                })
                .collect();
            let max = values
                .iter()
                .flatten()
                .copied()
                .filter(|v| v.is_finite())
                .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
            if let (true, Some(max)) = (legend.is_empty(), max) {
                legend.push(LegendItem {
                    text: t.get1("mail-chart-max", "value", format.format(t, ch.unit, max)),
                    tone: None,
                });
            }
            (
                t.msg(&channel_label(&ch.key)),
                view::value_bars(&values, &statuses, &ch.thresholds, height),
                legend,
                values,
            )
        }
    };
    Ok(Drawn {
        history: History {
            from_ms,
            to_ms: now_ms,
            title: t.get1("mail-chart-title", "channel", title_channel.as_str()),
            bars,
            legend,
            png,
            channel: title_channel,
        },
        values,
        bucket_ms,
    })
}

/// Mean of the bucket averages.
fn average(values: &[Option<f64>]) -> Option<f64> {
    let known: Vec<f64> = values
        .iter()
        .flatten()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    (!known.is_empty()).then(|| known.iter().sum::<f64>() / known.len() as f64)
}

/// The change over the last 2 hours (first to last known bucket in that time) and the lowest and
/// highest bucket before it. `None` without at least two values in the 2 hours and one before.
fn trend(values: &[Option<f64>], bucket_ms: i64) -> Option<(f64, f64, f64)> {
    let recent_buckets = usize::try_from(TREND_MS / bucket_ms.max(1)).ok()?;
    let split = values.len().checked_sub(recent_buckets)?;
    let known = |part: &[Option<f64>]| -> Vec<f64> {
        part.iter()
            .flatten()
            .copied()
            .filter(|v| v.is_finite())
            .collect()
    };
    let (earlier, recent) = (known(&values[..split]), known(&values[split..]));
    if recent.len() < 2 || earlier.is_empty() {
        return None;
    }
    let delta = recent[recent.len() - 1] - recent[0];
    let lo = earlier.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = earlier.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Some((delta, lo, hi))
}

/// "62–66 °C": two values with one unit when they share it.
fn range_text(t: Lang, format: &ValueFormat, unit: Unit, lo: f64, hi: f64) -> String {
    let (a, b) = (format.format(t, unit, lo), format.format(t, unit, hi));
    if a == b {
        return a;
    }
    match b.split_once(' ') {
        Some((_, suffix)) if a.ends_with(&format!(" {suffix}")) => {
            format!("{}–{b}", &a[..a.len() - suffix.len() - 1])
        }
        _ => format!("{a} – {b}"),
    }
}

/// "+9 °C", "−0,4 ms"
fn signed(t: Lang, format: &ValueFormat, unit: Unit, delta: f64) -> String {
    let sign = if delta < 0.0 { '−' } else { '+' };
    format!("{sign}{}", format.format(t, unit, delta.abs()))
}

/// The sensor answers requests: its recovery reads "answers again".
fn answers(spec: &SensorSpec) -> bool {
    matches!(
        spec,
        SensorSpec::Ping(_) | SensorSpec::Http(_) | SensorSpec::Tcp(_) | SensorSpec::Dns(_)
    )
}

/// Up to [`MAX_OTHERS`] other enabled sensors of the device, in display order, with their latest
/// value and status.
async fn others(pool: &SqlitePool, t: Lang, device_id: i64, sensor_id: i64) -> Result<Vec<Other>> {
    let mut out = Vec::new();
    for sensor in crate::storage::sensors::list_sensors(pool, device_id).await? {
        if out.len() >= MAX_OTHERS {
            break;
        }
        if !sensor.enabled || sensor.id == sensor_id {
            continue;
        }
        let status = crate::storage::status::get_state(pool, sensor.id)
            .await?
            .map_or(Status::Unknown, |s| s.status);
        let latest = crate::storage::measurements::latest_values(pool, &sensor.channels).await?;
        let value = main_channel(&sensor, &latest)
            .and_then(|ch| {
                let (_, v) = latest.get(&ch.id)?;
                let format = value_format(&sensor, ch);
                let labelled = format.labels.iter().any(|(n, _)| *n as f64 == *v);
                (ch.unit != Unit::State || labelled).then(|| format.format(t, ch.unit, *v))
            })
            .unwrap_or_else(|| "–".to_owned());
        out.push(Other {
            name: sensor.name,
            value,
            status,
        });
    }
    Ok(out)
}

/// Devices with how many of their sensors, by name, most sensors first.
fn dependents(per_device: BTreeMap<i64, usize>, devices: &HashMap<i64, &Device>) -> Vec<Dependent> {
    let mut out: Vec<Dependent> = per_device
        .into_iter()
        .map(|(id, sensors)| {
            let device = devices.get(&id);
            Dependent {
                name: device.map(|d| d.name.clone()).unwrap_or_default(),
                host: device.map(|d| d.host.clone()).unwrap_or_default(),
                sensors,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.sensors
            .cmp(&a.sensors)
            .then_with(|| crate::ordering::natural_cmp(&a.name, &b.name))
    });
    out
}

/// The details for each event (same order). A single change gets all of them; in a batch the first
/// [`MINI_ROWS`] changes (in the order the mail lists them) get a small history, the others only
/// what needs no query.
pub async fn gather(
    pool: &SqlitePool,
    t: Lang,
    pauses: &Pauses,
    events: &[AlertEvent],
    now_ms: i64,
) -> Vec<Details> {
    let single = events.len() == 1;
    let device_list = soft(
        crate::storage::devices::list_devices(pool).await,
        "devices",
        0,
    )
    .unwrap_or_default();
    let groups =
        soft(crate::storage::groups::list_groups(pool).await, "groups", 0).unwrap_or_default();
    let devices: HashMap<i64, &Device> = device_list.iter().map(|d| (d.id, d)).collect();
    let mut by_order: Vec<usize> = (0..events.len()).collect();
    by_order.sort_by_key(|&i| (order(events[i].change.to), events[i].change.ts_ms));
    let with_history: Vec<usize> = by_order.into_iter().take(MINI_ROWS).collect();

    let mut out = Vec::with_capacity(events.len());
    for (i, event) in events.iter().enumerate() {
        let device = devices.get(&event.device_id).copied();
        let mut details = Details {
            group_path: device
                .map(|d| group_path(&groups, d.group_id))
                .unwrap_or_default(),
            host: device.map(|d| d.host.clone()).unwrap_or_default(),
            ..Details::default()
        };
        if single || with_history.contains(&i) {
            one(
                pool,
                t,
                pauses,
                event,
                device,
                &devices,
                now_ms,
                single,
                &mut details,
            )
            .await;
        }
        out.push(details);
    }
    out
}

/// Fills in what can be read for one change.
#[allow(clippy::too_many_arguments)]
async fn one(
    pool: &SqlitePool,
    t: Lang,
    pauses: &Pauses,
    event: &AlertEvent,
    device: Option<&Device>,
    devices: &HashMap<i64, &Device>,
    now_ms: i64,
    single: bool,
    details: &mut Details,
) {
    let id = event.change.sensor_id;
    let sensor = soft(
        crate::storage::sensors::get_sensor(pool, id).await,
        "sensor",
        id,
    )
    .flatten();
    let interval = sensor.as_ref().map_or(DEFAULT_INTERVAL_SECS, |s| {
        s.interval(device.map_or(DEFAULT_INTERVAL_SECS, |d| d.polling.interval_secs))
    });
    let latest = match (&sensor, single) {
        (Some(sensor), true) => soft(
            crate::storage::measurements::latest_values(pool, &sensor.channels).await,
            "latest values",
            id,
        )
        .unwrap_or_default(),
        _ => BTreeMap::new(),
    };
    let channel = sensor.as_ref().and_then(|s| main_channel(s, &latest));
    let mut drawn = None;
    if let Some(sensor) = &sensor {
        drawn = soft(
            history(
                pool,
                t,
                sensor,
                channel,
                interval,
                now_ms,
                !single,
                single.then_some(event.change.to),
            )
            .await,
            "history",
            id,
        );
        details.history = drawn.as_ref().map(|d| d.history.clone());
        details.answers = answers(&sensor.spec);
    }
    if !single {
        return;
    }
    let warning = event.change.to == Status::Warning;
    details.previous_ms = soft(
        crate::storage::status::previous_change(pool, id, event.change.ts_ms).await,
        "previous change",
        id,
    )
    .flatten();
    if let (Some(sensor), Some(ch)) = (&sensor, channel) {
        let format = value_format(sensor, ch);
        if let Some((ts, value)) = latest.get(&ch.id) {
            let labelled = format.labels.iter().any(|(n, _)| *n as f64 == *value);
            // A bare status code (e.g. "1" for an interface) tells nothing: left out.
            if ch.unit != Unit::State || labelled {
                details.value = Some((format.format(t, ch.unit, *value), *ts));
            }
        }
        if ch.unit != Unit::State {
            details.limits = limits(t, &format, ch.unit, &ch.thresholds)
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>()
                .join(" · ");
            let breach = latest
                .get(&ch.id)
                .and_then(|(_, v)| ch.thresholds.breach(*v));
            details.beyond = breach.map(|b| match b {
                Breach::WarnAbove(l) | Breach::DownAbove(l) => {
                    t.get1("mail-beyond-above", "value", format.format(t, ch.unit, l))
                }
                Breach::WarnBelow(l) | Breach::DownBelow(l) => {
                    t.get1("mail-beyond-below", "value", format.format(t, ch.unit, l))
                }
            });
            if let Some(drawn) = drawn.as_ref().filter(|d| !d.values.is_empty()) {
                details.average = average(&drawn.values).map(|v| format.format(t, ch.unit, v));
                if warning {
                    details.trend = trend(&drawn.values, drawn.bucket_ms).map(|(delta, lo, hi)| {
                        (
                            signed(t, &format, ch.unit, delta),
                            range_text(t, &format, ch.unit, lo, hi),
                        )
                    });
                }
            }
            if warning {
                // Before the change, so the value that tripped the warning is not its own peak.
                let below = matches!(breach, Some(Breach::WarnBelow(_) | Breach::DownBelow(_)));
                let extreme = crate::storage::measurements::extreme(
                    pool,
                    ch.id,
                    now_ms - WEEK_MS,
                    event.change.ts_ms,
                    !below,
                )
                .await;
                if let Some((ts, v)) = soft(extreme, "peak", id).flatten() {
                    let label = if below {
                        "mail-fact-low"
                    } else {
                        "mail-fact-peak"
                    };
                    details.peak = Some((label, format.format(t, ch.unit, v), ts));
                }
            }
        }
    }
    if warning {
        details.others = soft(
            others(pool, t, event.device_id, id).await,
            "other sensors",
            id,
        )
        .unwrap_or_default();
    }
    let week = now_ms - WEEK_MS;
    if let Some(totals) = soft(
        crate::storage::availability::totals(pool, week, now_ms, Some(id)).await,
        "availability",
        id,
    ) {
        let totals = totals.get(&id).copied().unwrap_or_default();
        details.availability = totals
            .percent()
            .map(|p| crate::web::reports::percent_text(t, p));
        if let Some(count) = soft(
            crate::storage::status::count_changes_to(pool, id, Status::Down, week).await,
            "outages",
            id,
        ) {
            let total = view::duration(t, totals.down_ms);
            details.outages = Some((count, total));
        }
    }
    details.dependents = match event.change.to {
        Status::Down => dependents(pauses.paused_through(id), devices),
        Status::Up => dependents(pauses.dependents(id), devices),
        _ => Vec::new(),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_group_path_runs_from_the_top() {
        let group = |id, name: &str, parent| Group {
            id,
            name: name.to_owned(),
            parent_id: parent,
        };
        let groups = [
            group(1, "Haus", None),
            group(2, "Netzwerk", Some(1)),
            group(3, "Kreis", Some(3)),
        ];
        assert_eq!(group_path(&groups, Some(2)), ["Haus", "Netzwerk"]);
        assert!(group_path(&groups, None).is_empty());
        assert!(group_path(&groups, Some(3)).len() <= groups.len() + 1);
    }

    #[test]
    fn trend_average_and_ranges() {
        // 72 buckets of 5 minutes: the last 24 are the last 2 hours.
        let mut values: Vec<Option<f64>> = (0..48).map(|i| Some(62.0 + (i % 5) as f64)).collect();
        values.extend((0..24).map(|i| (i != 5).then_some(69.0 + f64::from(i) / 2.5)));
        let (delta, lo, hi) = trend(&values, 300_000).unwrap();
        assert!(
            (delta - 9.2).abs() < 1e-9 && lo == 62.0 && hi == 66.0,
            "{delta} {lo} {hi}"
        );
        assert_eq!(trend(&values[48..], 300_000), None, "nothing before");
        assert_eq!(trend(&[None, Some(1.0), Some(2.0)], 300_000), None);
        assert_eq!(average(&[Some(1.0), None, Some(2.0)]), Some(1.5));
        assert_eq!(average(&[None]), None);
        let f = ValueFormat::default();
        assert_eq!(
            range_text(Lang::De, &f, Unit::Celsius, 62.0, 66.0),
            "62–66 °C"
        );
        assert_eq!(range_text(Lang::De, &f, Unit::Celsius, 62.0, 62.0), "62 °C");
        assert_eq!(signed(Lang::De, &f, Unit::Celsius, 9.2), "+9,2 °C");
        assert_eq!(signed(Lang::De, &f, Unit::Milliseconds, -0.5), "−0,5 ms");
    }

    #[test]
    fn slow_sensors_get_longer_buckets() {
        assert_eq!(buckets(BUCKET_MS, 60), (300_000, 72));
        assert_eq!(buckets(BUCKET_MS, 300), (600_000, 36));
        assert_eq!(buckets(MINI_BUCKET_MS, 60), (900_000, 24));
    }

    #[test]
    fn limits_are_named_with_their_unit() {
        let thresholds = Thresholds {
            warn_above: Some(75.0),
            down_above: Some(90.0),
            ..Thresholds::default()
        };
        let texts = limits(
            Lang::De,
            &ValueFormat::default(),
            Unit::Celsius,
            &thresholds,
        );
        assert_eq!(
            texts,
            [
                (Tone::Warning, "Warnung über 75 °C".to_owned()),
                (Tone::Down, "Ausfall über 90 °C".to_owned())
            ]
        );
    }
}
