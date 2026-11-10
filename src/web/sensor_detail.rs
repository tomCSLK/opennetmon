//! Sensor detail page: current values, chart (uPlot, data from a JSON endpoint), events. Name,
//! interval, settings, thresholds and ratings are changed on the settings page (`web::sensor_settings`).

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::{Form, Json};
use serde::{Deserialize, Serialize};

use crate::audit::{Action, Changes, Kind};
use crate::devices::Device;
use crate::error::{Error, Result};
use crate::i18n::{Lang, Msg};
use crate::sensors::Unit;
use crate::sensors::format::{channel_label, parse_threshold, threshold_input};
use crate::sensors::spec::SensorSpec;
use crate::status::{Status, Thresholds, ValueState};
use crate::storage::devices;
use crate::storage::measurements::{ChartSeries, Range, chart_data, latest_values, recent_samples};
use crate::storage::sensors::{self as store, ChannelRecord, SensorRecord};
use crate::storage::status as status_store;
use crate::web::audit::{self, Object};
use crate::web::auth::CurrentUser;
use crate::web::sensor_text::ValueFormat;
use crate::web::{AppState, Crumb, Layout, flash_message, group_trail, time_html};

/// Status changes in the first page of the sensor page; the script then takes over.
const EVENT_LIMIT: u32 = 20;
/// Page sizes the events endpoint serves (the first one is the default).
const EVENT_PAGE_SIZES: [u32; 3] = [20, 50, 100];
/// Safety bound for the table of values (7 days at a 60-second interval are about 10,000 rows).
const TABLE_ROWS_MAX: u32 = 20_000;

#[derive(Deserialize)]
pub struct RangeQuery {
    range: Option<String>,
    msg: Option<String>,
}

impl RangeQuery {
    fn range(&self) -> Range {
        self.range
            .as_deref()
            .and_then(Range::parse)
            .unwrap_or(Range::Day)
    }
}

/// A tile of the sensor page: a latest value or an availability.
struct ValueTile {
    /// Symbol before the label (`web::icon` name), empty for none.
    icon: &'static str,
    label: String,
    value: String,
    /// The line under the value: a `<time>` (markup) or plain text.
    when_html: String,
    when_text: String,
}

/// A row of tiles; `label` names it for screen readers.
struct ValueRow {
    label: String,
    tiles: Vec<ValueTile>,
}

/// At most this many tiles stand in a row.
const TILES_PER_ROW: usize = 5;

/// The latest values and the availability in one row when they fit, as in the draft; otherwise
/// the values in rows of at most [`TILES_PER_ROW`] (shared out evenly) and the availability below.
fn value_rows(t: Lang, values: Vec<ValueTile>, availability: Vec<ValueTile>) -> Vec<ValueRow> {
    if values.len() + availability.len() <= TILES_PER_ROW {
        let label = format!(
            "{} · {}",
            t.get("detail-latest"),
            t.get("report-availability")
        );
        let tiles = values.into_iter().chain(availability).collect();
        return vec![ValueRow { label, tiles }];
    }
    let rows = values.len().div_ceil(TILES_PER_ROW).max(1);
    let per_row = values.len().div_ceil(rows).max(1);
    let mut out = Vec::new();
    let mut values = values.into_iter().peekable();
    while values.peek().is_some() {
        out.push(ValueRow {
            label: t.get("detail-latest"),
            tiles: values.by_ref().take(per_row).collect(),
        });
    }
    if !availability.is_empty() {
        out.push(ValueRow {
            label: t.get("report-availability"),
            tiles: availability,
        });
    }
    out
}

/// The symbol of a channel tile: arrows for the two directions, a pulse for a lone value.
fn channel_icon(key: &str, lone: bool) -> &'static str {
    match key {
        "in" | "sync_down" => "arrow-down",
        "out" | "sync_up" => "arrow-up",
        _ if lone => "pulse",
        _ => "",
    }
}

pub(crate) struct StateRow {
    pub value: i64,
    pub label: String,
    /// `up`, `warning` or `down`.
    pub status: &'static str,
}

/// The rating form for sensors whose values have names (MIB enumeration).
pub(crate) struct StatesView {
    pub rows: Vec<StateRow>,
    /// Ratings are stored; otherwise the rows show a suggestion (or the interface default).
    pub stored: bool,
    pub interface: bool,
}

/// A choice how far back the table of values reaches.
struct SpanOption {
    minutes: u32,
    label: String,
}

struct RangeLink {
    value: &'static str,
    label: String,
    current: bool,
}

struct EventView {
    at: String,
    change: String,
    message: String,
    /// `up`, `warning`, `down` or `paused`; warnings and outages are tinted.
    status: &'static str,
}

/// One status change as the page and the events endpoint show it.
fn event_view(t: Lang, e: &status_store::Event) -> EventView {
    EventView {
        at: time_html(e.ts_ms),
        change: event_change(t, e),
        message: crate::status::shown(t, &e.message),
        status: e.to.as_str(),
    }
}

fn event_change(t: Lang, e: &status_store::Event) -> String {
    match e.from {
        Some(from) => format!("{} → {}", t.get(from.label_id()), t.get(e.to.label_id())),
        None => t.get1("event-first-poll", "status", t.get(e.to.label_id())),
    }
}

#[derive(Template)]
#[template(path = "sensor_detail.html")]
struct DetailTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    sensor: SensorRecord,
    description: String,
    device: Device,
    /// Groups above the device, for the breadcrumbs.
    trail: Vec<Crumb>,
    status_label: String,
    status_class: &'static str,
    message: String,
    /// Why the sensor is paused, instead of the message.
    pause: Option<crate::web::pause::PauseNote>,
    /// Paused by itself, so it can be resumed here.
    paused_itself: bool,
    /// What the sensor depends on, e.g. "abhängig von „Ping“ (Router)"; empty without dependency.
    dependency: Vec<String>,
    last_poll: String,
    /// The tiles of latest values and availability, in rows.
    value_rows: Vec<ValueRow>,
    ranges: Vec<RangeLink>,
    /// How far back the table can reach in this range; no choice when there is only one.
    spans: Vec<SpanOption>,
    range: &'static str,
    events: Vec<EventView>,
    /// Status changes kept for the sensor.
    events_total: i64,
    flash: Option<String>,
    /// The interval the sensor is polled at (its own or the device's).
    interval: u32,
    /// The ratings of named status values, shown to readers (admins change them in the settings).
    states: Option<StatesView>,
    /// Status values only (link status): a band of the status over the range instead of a chart.
    band: bool,
    /// The range shown, e.g. "24 Stunden" (card head of the chart).
    range_label: String,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

pub(crate) async fn load(state: &AppState, id: i64) -> Result<(SensorRecord, Device)> {
    let sensor = store::get_sensor(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)?;
    let device = devices::get_device(&state.pool, sensor.device_id)
        .await?
        .ok_or(Error::NotFound)?;
    Ok((sensor, device))
}

pub(crate) fn fields_of(lang: Lang, t: &Thresholds, unit: Unit) -> [String; 4] {
    [t.warn_above, t.warn_below, t.down_above, t.down_below].map(|v| threshold_input(lang, v, unit))
}

async fn render(
    state: &AppState,
    user: &CurrentUser,
    sensor: SensorRecord,
    device: Device,
    range: Range,
    flash: Option<String>,
) -> Result<String> {
    let latest = latest_values(&state.pool, &sensor.channels).await?;
    let mibs = state.mib_index();
    let value_format = ValueFormat::for_sensor(&sensor.spec, &mibs);
    let states = states_view(&sensor, &value_format);
    let lone = sensor.channels.len() == 1;
    let channels = sensor
        .channels
        .iter()
        .map(|c| ValueTile {
            icon: channel_icon(&c.key, lone),
            label: user.lang.msg(&channel_label(&c.key)),
            value: latest.get(&c.id).map_or_else(
                || "–".to_owned(),
                |(_, v)| value_format.format(user.lang, c.unit, *v),
            ),
            when_html: latest
                .get(&c.id)
                .map_or_else(String::new, |(ts, _)| time_html(*ts)),
            when_text: user.lang.get("detail-no-value"),
        })
        .collect();
    let state_row = status_store::get_state(&state.pool, sensor.id).await?;
    let pauses = crate::storage::pauses::load_all(&state.pool, now_ms()).await?;
    let why = pauses.sensor(sensor.id, device.id);
    let dependency = dependency_lines(
        user.lang,
        &pauses,
        sensor.id,
        why.as_ref().and_then(|w| w.dependency()),
    );
    let status = crate::status::shown_status(why.is_some(), state_row.as_ref().map(|s| s.status));
    let events = status_store::events_of_sensor(&state.pool, sensor.id, EVENT_LIMIT, 0)
        .await?
        .iter()
        .map(|e| event_view(user.lang, e))
        .collect();
    let (events_total, _) = status_store::event_totals(&state.pool, sensor.id).await?;
    let availability = crate::web::reports::sensor_summary(state, user.lang, sensor.id)
        .await?
        .into_iter()
        .map(|(period, value, down)| ValueTile {
            icon: "",
            label: user
                .lang
                .get1("report-availability-of", "period", period.as_str()),
            value,
            when_html: String::new(),
            when_text: if down.is_empty() {
                String::new()
            } else {
                user.lang.get1("report-downtime-of", "time", down.as_str())
            },
        })
        .collect();
    let page = DetailTemplate {
        value_rows: value_rows(user.lang, channels, availability),
        layout: user.layout(),
        t: user.lang,
        description: crate::web::sensor_text::describe(user.lang, &sensor.spec, &mibs),
        status_label: user.lang.get(status.label_id()),
        status_class: status.as_str(),
        message: state_row
            .as_ref()
            .filter(|_| why.is_none())
            .map(|s| crate::status::shown(user.lang, &s.message))
            .unwrap_or_default(),
        paused_itself: why
            .as_ref()
            .is_some_and(|w| w.source == crate::pause::Source::Own),
        pause: why.as_ref().map(|w| crate::web::pause::note(user.lang, w)),
        dependency,
        last_poll: state_row.map_or_else(String::new, |s| time_html(s.last_poll_ms)),
        ranges: Range::ALL
            .into_iter()
            .map(|r| RangeLink {
                value: r.as_str(),
                label: user.lang.get(r.label_id()),
                current: r == range,
            })
            .collect(),
        spans: range
            .table_spans()
            .iter()
            .map(|&minutes| SpanOption {
                minutes,
                label: user.lang.get1("detail-span", "minutes", minutes),
            })
            .collect(),
        range: range.as_str(),
        events,
        events_total,
        flash,
        interval: sensor.interval(device.polling.interval_secs),
        states,
        band: !sensor.channels.is_empty() && sensor.channels.iter().all(|c| c.unit == Unit::State),
        range_label: user.lang.get(range.label_id()),
        sensor,
        trail: group_trail(&state.pool, device.group_id).await?,
        device,
    };
    Ok(page.render()?)
}

/// The sensors a sensor depends on (its setting, own or inherited, and the master sensor of its
/// device), and whether it is the master itself; not the one that pauses it now (`pausing`), which
/// the pause note names already.
fn dependency_lines(
    t: Lang,
    pauses: &crate::pause::Pauses,
    id: i64,
    pausing: Option<i64>,
) -> Vec<String> {
    let Some(info) = pauses.sensor_info(id) else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    if info.master {
        lines.push(t.get("dependency-is-master"));
    }
    let on = pauses.effective(crate::pause::Target::Sensor(id)).sensor;
    let master = pauses.master_of(info.device_id);
    for target in [on, master]
        .into_iter()
        .flatten()
        .filter(|t| *t != id && Some(*t) != pausing)
    {
        if let Some(other) = pauses.sensor_info(target) {
            let line = t.get2(
                "dependency-depends-on",
                "sensor",
                other.name.as_str(),
                "device",
                other.device.as_str(),
            );
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
    }
    lines
}

/// The channel whose values can be rated: the single channel of an OID value with named values or
/// of an interface status sensor.
pub(crate) fn rated_channel(sensor: &SensorRecord) -> Option<&ChannelRecord> {
    match sensor.spec {
        SensorSpec::InterfaceStatus { .. } => sensor.channels.first(),
        SensorSpec::Oid { .. } => sensor.channels.first().filter(|c| c.unit == Unit::Count),
        _ => None,
    }
}

/// Named values: the MIB enumeration plus stored ratings of values the MIB does not (or no longer) list.
pub(crate) fn named_values(channel: &ChannelRecord, format: &ValueFormat) -> Vec<(i64, String)> {
    let mut values = format.labels.clone();
    for s in &channel.states {
        if !values.iter().any(|(v, _)| *v == s.value) {
            values.push((s.value, s.label.clone()));
        }
    }
    values.sort_by_key(|(v, _)| *v);
    values
}

pub(crate) fn states_view(sensor: &SensorRecord, format: &ValueFormat) -> Option<StatesView> {
    let channel = rated_channel(sensor)?;
    let values = named_values(channel, format);
    if values.is_empty() {
        return None;
    }
    let interface = matches!(sensor.spec, SensorSpec::InterfaceStatus { .. });
    let stored = !channel.states.is_empty();
    let rows = values
        .into_iter()
        .map(|(value, label)| {
            let status = match channel.states.iter().find(|s| s.value == value) {
                Some(s) => s.status,
                None if stored => Status::Warning, // unrated value of a rated channel
                None if interface => {
                    if value == 1 {
                        Status::Up
                    } else {
                        Status::Down
                    }
                }
                None => crate::status::suggest_status(&label),
            };
            StateRow {
                value,
                label,
                status: status.as_str(),
            }
        })
        .collect();
    Some(StatesView {
        rows,
        stored,
        interface,
    })
}

pub async fn detail(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<RangeQuery>,
) -> Result<Html<String>> {
    let (sensor, device) = load(&state, id).await?;
    Ok(Html(
        render(
            &state,
            &user,
            sensor,
            device,
            query.range(),
            flash_message(user.lang, query.msg.as_deref()),
        )
        .await?,
    ))
}

#[derive(Serialize)]
struct SeriesJson {
    key: String,
    label: String,
    /// Unit shown after the value (custom OID sensors), e.g. `°C`.
    unit_label: Option<String>,
    /// Enumeration labels from the MIB as `[value, "label"]` pairs.
    labels: Vec<(i64, String)>,
    unit: &'static str,
    /// A channel the table of values hides on phones (ping: minimum, maximum, jitter).
    minor: bool,
    values: Vec<Option<f64>>,
}

/// A stretch of time in one status (seconds), for the band of status sensors and the tinted
/// outages of the chart.
#[derive(Serialize)]
struct PeriodJson {
    from: i64,
    to: i64,
    status: &'static str,
    label: String,
}

#[derive(Serialize)]
pub struct ChartJson {
    range: &'static str,
    /// Start and end of the range (seconds); the chart spans them.
    from: i64,
    to: i64,
    /// The status of the sensor over the range, from its status changes.
    periods: Vec<PeriodJson>,
    /// Seconds covered by one point when points are averages (7 and 30 days); `None` for raw samples.
    step_secs: Option<i64>,
    /// Polling interval of the device, to estimate the time a raw sample covers.
    interval_secs: u32,
    ts: Vec<i64>,
    series: Vec<SeriesJson>,
}

/// Chart data for uPlot: aligned timestamps (seconds) and one value list per channel.
pub async fn data(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<RangeQuery>,
) -> Result<Json<ChartJson>> {
    let (sensor, device) = load(&state, id).await?;
    let range = query.range();
    let now = now_ms();
    let chart = chart_data(&state.pool, &sensor.channels, range, now).await?;
    let from = now - range.duration_ms();
    let (start, changes) = status_store::status_history(&state.pool, sensor.id, from).await?;
    let periods = crate::web::sparkline::periods(start, &changes, from, now)
        .into_iter()
        .map(|(a, b, status)| PeriodJson {
            from: a.div_euclid(1000),
            to: b.div_euclid(1000),
            status: status.as_str(),
            label: user.lang.get(status.label_id()),
        })
        .collect();
    Ok(Json(ChartJson {
        range: range.as_str(),
        from: from.div_euclid(1000),
        to: now.div_euclid(1000),
        periods,
        step_secs: range.step_secs(),
        interval_secs: sensor.interval(device.polling.interval_secs),
        ts: chart.ts,
        series: series_json(&state, &sensor, user.lang, chart.series),
    }))
}

fn series_json(
    state: &AppState,
    sensor: &SensorRecord,
    lang: Lang,
    series: Vec<ChartSeries>,
) -> Vec<SeriesJson> {
    let value_format = ValueFormat::for_sensor(&sensor.spec, &state.mib_index());
    series
        .into_iter()
        .map(|s| SeriesJson {
            label: lang.msg(&channel_label(&s.key)),
            unit_label: value_format.unit_label.clone(),
            labels: value_format.labels.clone(),
            unit: s.unit.as_str(),
            minor: sensor.spec.minor(&s.key),
            key: s.key,
            values: s.values,
        })
        .collect()
}

#[derive(Deserialize)]
pub struct SamplesQuery {
    range: Option<String>,
    minutes: Option<u32>,
}

#[derive(Serialize)]
pub struct SamplesJson {
    /// Polling interval of the device, to estimate the time a raw sample covers.
    interval_secs: u32,
    /// Polls in the span; `ts` holds all of them up to [`TABLE_ROWS_MAX`], the newest.
    total: u64,
    ts: Vec<i64>,
    series: Vec<SeriesJson>,
    /// Per row: `"warning"` or `"down"` for a poll that did not end up (a failed poll has no
    /// values), else `null`.
    states: Vec<Option<&'static str>>,
    /// Per row: the message of such a poll, in the language of the reader.
    messages: Vec<Option<String>>,
}

/// The table of values: the raw samples (every poll) of the last `minutes`, however the chart
/// averages the range; `minutes` is one of [`Range::table_spans`], otherwise the first.
pub async fn samples(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<SamplesQuery>,
) -> Result<Json<SamplesJson>> {
    let (sensor, device) = load(&state, id).await?;
    let range = query
        .range
        .as_deref()
        .and_then(Range::parse)
        .unwrap_or(Range::Day);
    let spans = range.table_spans();
    let minutes = query
        .minutes
        .filter(|h| spans.contains(h))
        .or_else(|| spans.first().copied())
        .unwrap_or(1);
    let rows = recent_samples(
        &state.pool,
        sensor.id,
        &sensor.channels,
        minutes,
        now_ms(),
        TABLE_ROWS_MAX,
    )
    .await?;
    Ok(Json(SamplesJson {
        interval_secs: sensor.interval(device.polling.interval_secs),
        total: rows.total,
        ts: rows.data.ts,
        series: series_json(&state, &sensor, user.lang, rows.data.series),
        states: rows
            .problems
            .iter()
            .map(|p| p.as_ref().map(|p| p.status.as_str()))
            .collect(),
        messages: rows
            .problems
            .iter()
            .map(|p| {
                p.as_ref()
                    .map(|p| crate::status::shown(user.lang, &p.message))
            })
            .collect(),
    }))
}

#[derive(Deserialize)]
pub struct EventsQuery {
    offset: Option<u32>,
    limit: Option<u32>,
    /// Id of the event that was on top of the page shown before: the page then begins with it (or
    /// with the next older one if it is gone), however many changes were added since.
    anchor: Option<i64>,
}

#[derive(Serialize)]
pub struct EventsJson {
    /// Status changes kept for the sensor.
    total: i64,
    /// Time of the oldest one, in ms; `null` without any.
    oldest_ms: Option<i64>,
    /// Number of changes before the first one returned.
    offset: u32,
    events: Vec<EventJson>,
}

#[derive(Serialize)]
struct EventJson {
    id: i64,
    ts_ms: i64,
    change: String,
    message: String,
    status: &'static str,
}

/// The status changes of a sensor, newest first, one page at a time (`?offset=&limit=`, limit 20,
/// 50 or 100) for the script of the sensor page.
pub async fn events(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<EventsJson>> {
    let (sensor, _) = load(&state, id).await?;
    let limit = query
        .limit
        .filter(|l| EVENT_PAGE_SIZES.contains(l))
        .unwrap_or(EVENT_PAGE_SIZES[0]);
    let (total, oldest_ms) = status_store::event_totals(&state.pool, sensor.id).await?;
    let wanted = match query.anchor {
        Some(anchor) => {
            let newer = status_store::events_newer_than(&state.pool, sensor.id, anchor).await?;
            u32::try_from(newer).unwrap_or(u32::MAX)
        }
        None => query.offset.unwrap_or(0),
    };
    // Past the end: the last page, so a page that lost its rows to the retention limit is not empty.
    let last_page = u32::try_from((total - 1).max(0)).unwrap_or(u32::MAX) / limit * limit;
    let offset = wanted.min(last_page);
    let events = status_store::events_of_sensor(&state.pool, sensor.id, limit, offset)
        .await?
        .iter()
        .map(|e| EventJson {
            id: e.id,
            ts_ms: e.ts_ms,
            change: event_change(user.lang, e),
            message: crate::status::shown(user.lang, &e.message),
            status: e.to.as_str(),
        })
        .collect();
    Ok(Json(EventsJson {
        total,
        oldest_ms,
        offset,
        events,
    }))
}

async fn audit_changed(
    state: &AppState,
    user: &CurrentUser,
    sensor: &SensorRecord,
    name: &str,
    changes: Changes,
) {
    audit::record(
        state,
        user,
        Action::Changed,
        Object::new(Kind::Sensor, sensor.id, name),
        changes,
    )
    .await;
}

/// Parses the threshold fields of every channel from the submitted form.
pub(crate) fn parse_limits(
    channels: &[ChannelRecord],
    form: &HashMap<String, String>,
) -> Result<Vec<(i64, Thresholds)>> {
    channels
        .iter()
        .filter(|c| c.unit != Unit::State)
        .map(|c| {
            let field = |prefix: &str| {
                let text = form
                    .get(&format!("{prefix}_{}", c.id))
                    .map_or("", String::as_str);
                parse_threshold(text, c.unit).map_err(Error::invalid)
            };
            let limits = Thresholds {
                warn_above: field("wa")?,
                warn_below: field("wb")?,
                down_above: field("da")?,
                down_below: field("db")?,
            };
            limits.validate().map_err(|m| {
                Error::invalid(
                    Msg::new("channel-error")
                        .with("channel", channel_label(&c.key))
                        .with("message", m),
                )
            })?;
            Ok((c.id, limits))
        })
        .collect()
}

/// The changed ratings of status values for the audit log.
pub(crate) fn states_changes(old: &[ValueState], new: &[ValueState]) -> Changes {
    let rating = |list: &[ValueState], value: i64| {
        list.iter()
            .find(|s| s.value == value)
            .map_or("audit-not-rated", |s| s.status.label_id())
    };
    let mut seen = std::collections::HashSet::new();
    let mut changes = Changes::new();
    for value in old.iter().chain(new) {
        if !seen.insert(value.value) {
            continue;
        }
        let (before, after) = (rating(old, value.value), rating(new, value.value));
        if before != after {
            changes = changes.note(
                Msg::new("audit-change")
                    .with("field", value.label.as_str())
                    .with("old", Msg::new(before))
                    .with("new", Msg::new(after)),
            );
        }
    }
    changes
}

/// Saves (`action=save`, fields `st_<value>` = `up`/`warning`/`down`) or removes (`action=reset`)
/// the ratings of the sensor's status values.
pub async fn save_states(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(form.get("csrf_token").map_or("", String::as_str))?;
    let (sensor, _) = load(&state, id).await?;
    let format = ValueFormat::for_sensor(&sensor.spec, &state.mib_index());
    let channel = rated_channel(&sensor).ok_or_else(|| Error::invalid(Msg::new("states-none")))?;
    let states = match form.get("action").map(String::as_str) {
        Some("reset") => Vec::new(),
        Some("save") => named_values(channel, &format)
            .into_iter()
            .map(|(value, label)| {
                let status = match form.get(&format!("st_{value}")).map(String::as_str) {
                    Some("up") => Status::Up,
                    Some("warning") => Status::Warning,
                    Some("down") => Status::Down,
                    _ => {
                        return Err(Error::invalid(
                            Msg::new("states-choose").with("value", label.as_str()),
                        ));
                    }
                };
                Ok(ValueState {
                    value,
                    label,
                    status,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        _ => return Err(Error::invalid(Msg::new("unknown-action"))),
    };
    if states.is_empty() && form.get("action").map(String::as_str) == Some("save") {
        return Err(Error::invalid(Msg::new("states-none")));
    }
    store::set_value_states(&state.pool, id, channel.id, &states).await?;
    tracing::info!(user = %user.username, sensor = id, rated = states.len(), "status value ratings saved");
    let changes = states_changes(&channel.states, &states);
    audit_changed(&state, &user, &sensor, &sensor.name, changes).await;
    let msg = if states.is_empty() {
        "states_reset"
    } else {
        "states_saved"
    };
    Ok(Redirect::to(&format!("/sensors/{id}?msg={msg}")).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{DeviceInput, Polling};

    fn tiles(n: usize) -> Vec<ValueTile> {
        (0..n)
            .map(|i| ValueTile {
                icon: "",
                label: i.to_string(),
                value: String::new(),
                when_html: String::new(),
                when_text: String::new(),
            })
            .collect()
    }

    /// Up to five tiles share one row; otherwise the values fill rows of at most five, shared out
    /// evenly, and the availability follows in a row of its own.
    #[test]
    fn tiles_stand_in_rows_of_at_most_five() {
        let sizes = |values, availability| -> Vec<usize> {
            value_rows(Lang::De, tiles(values), tiles(availability))
                .iter()
                .map(|r| r.tiles.len())
                .collect()
        };
        assert_eq!(sizes(1, 3), [4]);
        assert_eq!(sizes(2, 3), [5]);
        assert_eq!(sizes(3, 3), [3, 3]);
        assert_eq!(sizes(5, 3), [5, 3]);
        assert_eq!(sizes(7, 3), [4, 3, 3]);
        assert_eq!(sizes(11, 3), [4, 4, 3, 3]);
        assert_eq!(sizes(0, 3), [3]);
        assert_eq!(sizes(6, 0), [3, 3]);
    }

    #[test]
    fn directions_have_arrows_and_a_lone_value_a_pulse() {
        assert_eq!(channel_icon("in", false), "arrow-down");
        assert_eq!(channel_icon("sync_up", false), "arrow-up");
        assert_eq!(channel_icon("rtt", true), "pulse");
        assert_eq!(channel_icon("rtt", false), "");
        // Every symbol it names exists (an unknown name would be drawn as a dot).
        let dot = crate::web::icon_shapes("nope");
        for name in ["in", "out", "sync_down", "sync_up"]
            .map(|k| channel_icon(k, false))
            .into_iter()
            .chain([channel_icon("rtt", true)])
        {
            assert_ne!(crate::web::icon_shapes(name), dot, "{name}");
        }
    }
    use crate::sensors::spec::SensorSpec;
    use crate::status::Assessment;
    use crate::storage::devices::save_device;
    use crate::storage::sensors::{Sample, create_sensor, get_sensor, insert_samples};
    use crate::storage::status::record_poll;
    use crate::web::testutil::*;
    use axum::http::{StatusCode, header};

    /// A traffic sensor with samples; returns (sensor id, [in channel, out channel]).
    async fn traffic_sensor(h: &Harness) -> (i64, Vec<i64>) {
        let profile = add_v2c_profile(h, "P", "public").await;
        let device = save_device(
            &h.state.pool,
            None,
            DeviceInput {
                kind: Default::default(),
                name: "Switch".to_owned(),
                host: "192.0.2.1".to_owned(),
                group_id: None,
                profile_id: Some(profile),
                polling: Polling::default(),
            },
        )
        .await
        .unwrap();
        let sensor = create_sensor(
            &h.state.pool,
            device,
            "Traffic Port 1",
            &SensorSpec::InterfaceTraffic { if_index: 49153 },
        )
        .await
        .unwrap();
        let channels: Vec<i64> = get_sensor(&h.state.pool, sensor)
            .await
            .unwrap()
            .unwrap()
            .channels
            .iter()
            .map(|c| c.id)
            .collect();
        let now = now_ms();
        let samples = [
            Sample {
                channel_id: channels[0],
                ts_ms: now - 20_000,
                value: 12_500_000.0,
            },
            Sample {
                channel_id: channels[1],
                ts_ms: now - 20_000,
                value: 3_000.0,
            },
        ];
        insert_samples(&h.state.pool, &samples).await.unwrap();
        record_poll(
            &h.state.pool,
            sensor,
            now - 20_000,
            &[],
            &Assessment::Healthy("OK".into()),
        )
        .await
        .unwrap();
        (sensor, channels)
    }

    #[tokio::test]
    async fn the_breadcrumbs_lead_back_through_the_groups() {
        use crate::storage::groups::create_group;
        let h = harness(true).await;
        let (sensor, _) = traffic_sensor(&h).await;
        let (viewer, _) = login(&h, "bert").await;
        let crumbs = |body: &str| {
            let start = body.find("<div class=\"breadcrumb\">").unwrap();
            body[start..start + body[start..].find("</div>").unwrap()].to_owned()
        };

        // Without a group: overview, device, sensor.
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&viewer))).await;
        assert_eq!(
            crumbs(&page.body),
            "<div class=\"breadcrumb\"><a href=\"/\">Übersicht</a><span>›</span>\
             <a href=\"/devices/1/sensors\">Switch</a><span>›</span><span>Traffic Port 1</span>"
        );

        let network = create_group(&h.state.pool, "Netzwerk", None).await.unwrap();
        let routers = create_group(&h.state.pool, "Router", Some(network))
            .await
            .unwrap();
        sqlx::query("UPDATE devices SET group_id = ? WHERE id = 1")
            .bind(routers)
            .execute(&h.state.pool)
            .await
            .unwrap();
        let groups = format!(
            "<a href=\"/\">Übersicht</a><span>›</span><a href=\"/groups/{network}\">Netzwerk</a><span>›</span>\
             <a href=\"/groups/{routers}\">Router</a><span>›</span>"
        );
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&viewer))).await;
        assert_eq!(
            crumbs(&page.body),
            format!(
                "<div class=\"breadcrumb\">{groups}<a href=\"/devices/1/sensors\">Switch</a>\
                 <span>›</span><span>Traffic Port 1</span>"
            )
        );
        let page = send(&h.app, get("/devices/1/sensors", Some(&viewer))).await;
        assert_eq!(
            crumbs(&page.body),
            format!("<div class=\"breadcrumb\">{groups}<span>Switch</span>")
        );
        // The links lead to the group pages.
        for id in [network, routers] {
            let page = send(&h.app, get(&format!("/groups/{id}"), Some(&viewer))).await;
            assert_eq!(page.status, StatusCode::OK, "group {id}");
        }
    }

    #[tokio::test]
    async fn the_page_shows_latest_values_chart_hooks_and_events() {
        let h = harness(true).await;
        let (sensor, _) = traffic_sensor(&h).await;
        let (viewer, _) = login(&h, "bert").await;
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&viewer))).await;
        assert_eq!(page.status, StatusCode::OK);
        for needle in [
            "Traffic Port 1",
            "Eingehend",
            "12,5 Mbit/s",
            "3 kbit/s",
            "In Ordnung",
            "erste Abfrage: In Ordnung",
            "data-src=\"/sensors/1/data?range=24h\"",
            "/static/vendor/uplot/uPlot.iife.min.js",
            "/static/chart.js",
            "/static/time.js",
            "/static/pager.js",
            "/static/samples.js",
            "/static/events.js",
            "id=\"events\" class=\"table-wrap\" data-src=\"/sensors/1/events\"",
            "id=\"events-rows\"",
            "id=\"events-pager\"",
            "1 Wechsel",
            "id=\"samples\"",
            "data-src=\"/sensors/1/samples?range=24h\"",
            "id=\"samples-span\" data-range=\"24h\"",
            "id=\"samples-rows\"",
            "<option value=\"50\" selected>50</option>",
            "id=\"samples-pager\"",
            "<option value=\"60\" selected>letzte Stunde</option>",
            "<option value=\"1440\">letzte 24 Stunden</option>",
            "<time class=\"ts\" datetime=\"",
        ] {
            assert!(
                page.body.contains(needle),
                "{needle} missing: {}",
                page.body
            );
        }
        assert!(
            !page.body.contains(&format!("/sensors/{sensor}/edit")),
            "viewers get no settings"
        );

        let week = send(
            &h.app,
            get(&format!("/sensors/{sensor}?range=7d"), Some(&viewer)),
        )
        .await;
        assert!(week.body.contains("range=7d\"") && week.body.contains("aria-current=\"page\""));
        assert!(
            week.body
                .contains("<option value=\"360\" selected>letzte 6 Stunden</option>")
        );
        assert!(
            week.body
                .contains("<option value=\"10080\">letzte 7 Tage</option>")
        );
        // The hour offers quarter and half hours for short intervals.
        let hour = send(
            &h.app,
            get(&format!("/sensors/{sensor}?range=1h"), Some(&viewer)),
        )
        .await;
        assert!(
            hour.body
                .contains("<option value=\"15\" selected>letzte 15 Minuten</option>")
        );
        assert!(
            hour.body
                .contains("<option value=\"60\">letzte Stunde</option>")
        );
        let bogus = send(
            &h.app,
            get(&format!("/sensors/{sensor}?range=evil"), Some(&viewer)),
        )
        .await;
        assert!(
            bogus.body.contains("range=24h\""),
            "invalid ranges fall back to 24 hours"
        );
    }

    #[tokio::test]
    async fn the_json_endpoint_serves_aligned_series() {
        let h = harness(true).await;
        let (sensor, _) = traffic_sensor(&h).await;
        let (viewer, _) = login(&h, "bert").await;
        let reply = send(
            &h.app,
            get(&format!("/sensors/{sensor}/data?range=1h"), Some(&viewer)),
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK);
        assert!(
            reply.headers[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("application/json")
        );
        let body = &reply.body;
        assert!(body.contains("\"range\":\"1h\""), "{body}");
        assert!(
            body.contains("\"label\":\"Eingehend\"") && body.contains("\"unit\":\"bps\""),
            "{body}"
        );
        assert!(
            body.contains("12500000.0") && body.contains("3000.0"),
            "{body}"
        );
        assert!(
            body.contains("\"step_secs\":null") && body.contains("\"interval_secs\":60"),
            "{body}"
        );
        // The range the chart spans and the status over it, from the first poll on.
        assert!(
            body.contains("\"from\":")
                && body.contains("\"status\":\"up\",\"label\":\"In Ordnung\"}]"),
            "{body}"
        );
        crate::storage::status::record_poll(
            &h.state.pool,
            sensor,
            now_ms() - 10_000,
            &[],
            &Assessment::Failed("weg".into()),
        )
        .await
        .unwrap();
        let failed = send(
            &h.app,
            get(&format!("/sensors/{sensor}/data?range=1h"), Some(&viewer)),
        )
        .await;
        assert!(
            failed
                .body
                .contains("\"status\":\"warning\",\"label\":\"Warnung\"}]"),
            "a first failure warns, up to now: {}",
            failed.body
        );
        let week = send(
            &h.app,
            get(&format!("/sensors/{sensor}/data?range=7d"), Some(&viewer)),
        )
        .await;
        assert!(week.body.contains("\"step_secs\":5040"), "{}", week.body);

        assert_eq!(
            send(&h.app, get(&format!("/sensors/{sensor}/data"), None))
                .await
                .status,
            StatusCode::SEE_OTHER
        );
        assert_eq!(
            send(&h.app, get("/sensors/99/data", Some(&viewer)))
                .await
                .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(&h.app, get("/sensors/99", Some(&viewer))).await.status,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn the_table_endpoint_serves_raw_samples_in_every_range() {
        let h = harness(true).await;
        let (sensor, _) = traffic_sensor(&h).await;
        let (viewer, _) = login(&h, "bert").await;
        // The week's chart averages, the table still gets the poll itself.
        let reply = send(
            &h.app,
            get(
                &format!("/sensors/{sensor}/samples?range=7d&minutes=10080"),
                Some(&viewer),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK);
        let body = &reply.body;
        assert!(body.contains("\"total\":1"), "{body}");
        assert!(body.contains("\"interval_secs\":60"), "{body}");
        assert!(
            body.contains("[12500000.0]") && body.contains("[3000.0]"),
            "{body}"
        );
        assert!(
            body.contains("\"states\":[null]") && body.contains("\"messages\":[null]"),
            "{body}"
        );
        // A span the range does not offer falls back to its first instead of failing.
        let odd = send(
            &h.app,
            get(
                &format!("/sensors/{sensor}/samples?range=1h&minutes=10080"),
                Some(&viewer),
            ),
        )
        .await;
        assert_eq!(odd.status, StatusCode::OK);
        // A failed poll has no values but a row with its status and message.
        record_poll(
            &h.state.pool,
            sensor,
            now_ms() - 5_000,
            &[],
            &Assessment::Failed("no reply".into()),
        )
        .await
        .unwrap();
        let body = send(
            &h.app,
            get(
                &format!("/sensors/{sensor}/samples?range=1h"),
                Some(&viewer),
            ),
        )
        .await
        .body;
        assert!(body.contains("\"total\":2"), "{body}");
        assert!(
            body.contains("\"states\":[null,\"warning\"]")
                && body.contains("\"messages\":[null,\"no reply\"]")
                && body.contains("[12500000.0,null]"),
            "{body}"
        );
        assert_eq!(
            send(&h.app, get(&format!("/sensors/{sensor}/samples"), None))
                .await
                .status,
            StatusCode::SEE_OTHER
        );
        assert_eq!(
            send(&h.app, get("/sensors/99/samples", Some(&viewer)))
                .await
                .status,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn the_events_endpoint_serves_status_changes_page_by_page() {
        let h = harness(true).await;
        let (sensor, _) = traffic_sensor(&h).await;
        let (viewer, _) = login(&h, "bert").await;
        // The first poll wrote one change; 44 more flip between warning and up.
        let flip = |i: i64| {
            if i % 2 == 0 {
                Assessment::Failed("no reply".into())
            } else {
                Assessment::Healthy("OK".into())
            }
        };
        for i in 0..44 {
            record_poll(
                &h.state.pool,
                sensor,
                now_ms() - 100_000 + i * 1_000,
                &[],
                &flip(i),
            )
            .await
            .unwrap();
        }
        let fetch = |query: &'static str| {
            let h = &h;
            let viewer = &viewer;
            async move {
                let reply = send(
                    &h.app,
                    get(&format!("/sensors/{sensor}/events{query}"), Some(viewer)),
                )
                .await;
                assert_eq!(reply.status, StatusCode::OK, "{query}");
                serde_json::from_str::<serde_json::Value>(&reply.body).unwrap()
            }
        };

        let first = fetch("").await;
        assert_eq!(first["total"], 45);
        assert_eq!(first["offset"], 0);
        assert_eq!(first["events"].as_array().unwrap().len(), 20);
        assert!(first["oldest_ms"].is_i64());
        // Newest first: the last flip went back to up, the one before it warned.
        assert_eq!(first["events"][0]["status"], "up");
        assert_eq!(first["events"][1]["status"], "warning");
        assert_eq!(first["events"][1]["message"], "no reply");
        assert!(
            first["events"][1]["change"]
                .as_str()
                .unwrap()
                .contains(" → ")
        );

        // Only 20, 50 and 100 are page sizes.
        assert_eq!(
            fetch("?limit=7").await["events"].as_array().unwrap().len(),
            20
        );
        assert_eq!(
            fetch("?limit=50").await["events"].as_array().unwrap().len(),
            45
        );
        let last = fetch("?limit=20&offset=40").await;
        assert_eq!(
            (
                last["offset"].clone(),
                last["events"].as_array().unwrap().len()
            ),
            (40.into(), 5)
        );
        // Past the end: the last page instead of nothing.
        let past = fetch("?limit=20&offset=500").await;
        assert_eq!(
            (
                past["offset"].clone(),
                past["events"].as_array().unwrap().len()
            ),
            (40.into(), 5)
        );

        // A page is found again by the id of its top row, however many changes came in meanwhile.
        let second = fetch("?limit=20&offset=20").await;
        let top = second["events"][0]["id"].as_i64().unwrap();
        for i in 0..3 {
            record_poll(&h.state.pool, sensor, now_ms() + i * 1_000, &[], &flip(i))
                .await
                .unwrap();
        }
        let query: &'static str = Box::leak(format!("?limit=20&anchor={top}").into_boxed_str());
        let again = fetch(query).await;
        assert_eq!(again["offset"], 23);
        assert_eq!(again["events"][0]["id"].as_i64().unwrap(), top);

        // The sensor page itself draws the first page with warnings tinted.
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&viewer))).await;
        assert!(
            page.body.contains("<tr class=\"state-warning\">"),
            "{}",
            page.body
        );

        assert_eq!(
            send(&h.app, get(&format!("/sensors/{sensor}/events"), None))
                .await
                .status,
            StatusCode::SEE_OTHER
        );
        assert_eq!(
            send(&h.app, get("/sensors/99/events", Some(&viewer)))
                .await
                .status,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn admins_save_thresholds_in_the_display_unit() {
        let h = harness(true).await;
        let (sensor, channels) = traffic_sensor(&h).await;
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(
            &h.app,
            get(&format!("/sensors/{sensor}/edit"), Some(&admin)),
        )
        .await;
        assert!(
            page.body.contains(&format!("name=\"wa_{}\"", channels[0]))
                && page.body.contains("in Mbit/s")
        );

        let body = format!(
            "csrf_token={csrf}&wa_{0}=80%2C5&da_{0}=95&wb_{0}=&db_{0}=&wa_{1}=&wb_{1}=&da_{1}=&db_{1}=",
            channels[0], channels[1]
        );
        let reply = send(
            &h.app,
            post(&format!("/sensors/{sensor}/edit"), Some(&admin), &body),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/sensors/{sensor}?msg=sensor_settings_saved")
        );
        let stored = get_sensor(&h.state.pool, sensor).await.unwrap().unwrap();
        assert_eq!(stored.channels[0].thresholds.warn_above, Some(80_500_000.0));
        assert_eq!(stored.channels[0].thresholds.down_above, Some(95_000_000.0));
        assert!(stored.channels[1].thresholds.is_empty());

        let page = send(
            &h.app,
            get(
                &format!("/sensors/{sensor}?msg=sensor_settings_saved"),
                Some(&admin),
            ),
        )
        .await;
        assert!(page.body.contains("Einstellungen gespeichert."));
        let page = send(
            &h.app,
            get(&format!("/sensors/{sensor}/edit"), Some(&admin)),
        )
        .await;
        assert!(page.body.contains("value=\"80,5\""));
    }

    #[tokio::test]
    async fn the_settings_form_saves_everything_at_once_or_nothing() {
        let h = harness(true).await;
        let (sensor, channels) = traffic_sensor(&h).await;
        let (admin, csrf) = login(&h, "anna").await;
        let path = format!("/sensors/{sensor}/edit");
        let all = |name: &str, interval: &str, warn: &str| {
            format!(
                "csrf_token={csrf}&name={name}&interval={interval}&wa_{0}={warn}&wb_{0}=&da_{0}=&db_{0}=\
                 &wa_{1}=&wb_{1}=&da_{1}=&db_{1}=",
                channels[0], channels[1]
            )
        };
        // One field wrong: nothing is saved, every input is shown again.
        let page = send(
            &h.app,
            post(&path, Some(&admin), &all("Uplink", "30", "viel")),
        )
        .await;
        assert_eq!(page.status, StatusCode::UNPROCESSABLE_ENTITY);
        for part in ["value=\"Uplink\"", "value=\"30\"", "value=\"viel\""] {
            assert!(page.body.contains(part), "{part}");
        }
        let stored = get_sensor(&h.state.pool, sensor).await.unwrap().unwrap();
        assert_eq!(
            (stored.name.as_str(), stored.interval_secs),
            ("Traffic Port 1", None)
        );
        // All fine: everything saved, one entry in the audit log.
        let reply = send(
            &h.app,
            post(&path, Some(&admin), &all("Uplink", "30", "80")),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/sensors/{sensor}?msg=sensor_settings_saved")
        );
        let stored = get_sensor(&h.state.pool, sensor).await.unwrap().unwrap();
        assert_eq!(
            (stored.name.as_str(), stored.interval_secs),
            ("Uplink", Some(30))
        );
        assert_eq!(stored.channels[0].thresholds.warn_above, Some(80_000_000.0));
        let audit = crate::storage::audit::list(&h.state.pool, 10)
            .await
            .unwrap();
        let changed: Vec<_> = audit
            .iter()
            .filter(|e| e.action == crate::audit::Action::Changed)
            .collect();
        assert_eq!(changed.len(), 1);
        let text = Lang::De.msg(&changed[0].detail);
        assert!(
            text.contains("Name: Traffic Port 1 → Uplink")
                && text.contains("Intervall (s): – → 30")
                && text.contains("Warnung über"),
            "{text}"
        );
        // Saved again unchanged: no further entry.
        send(
            &h.app,
            post(&path, Some(&admin), &all("Uplink", "30", "80")),
        )
        .await;
        let audit = crate::storage::audit::list(&h.state.pool, 10)
            .await
            .unwrap();
        assert_eq!(
            audit
                .iter()
                .filter(|e| e.action == crate::audit::Action::Changed)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn bad_thresholds_are_explained_and_nothing_is_saved() {
        let h = harness(true).await;
        let (sensor, channels) = traffic_sensor(&h).await;
        let (admin, csrf) = login(&h, "anna").await;
        let path = format!("/sensors/{sensor}/edit");
        for (fields, expected) in [
            ("wa_{0}=abc".to_owned(), "Zahl"),
            ("wa_{0}=90&da_{0}=50".to_owned(), "Ausfallgrenze"),
            ("wa_{0}=10&wb_{0}=20".to_owned(), "unteren Grenze"),
        ] {
            let body = format!(
                "csrf_token={csrf}&{}",
                fields.replace("{0}", &channels[0].to_string())
            );
            let reply = send(&h.app, post(&path, Some(&admin), &body)).await;
            assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
            assert!(reply.body.contains(expected), "{body}: {}", reply.body);
            assert!(reply.body.contains("Eingehend:") || expected == "Zahl");
        }
        let echoed = send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!("csrf_token={csrf}&wa_{}=abc", channels[0]),
            ),
        )
        .await;
        assert!(
            echoed.body.contains("value=\"abc\""),
            "input stays in the form"
        );
        let stored = get_sensor(&h.state.pool, sensor).await.unwrap().unwrap();
        assert!(stored.channels.iter().all(|c| c.thresholds.is_empty()));
    }

    #[tokio::test]
    async fn only_admins_with_a_valid_token_may_save_thresholds() {
        let h = harness(true).await;
        let (sensor, channels) = traffic_sensor(&h).await;
        let path = format!("/sensors/{sensor}/edit");
        let (viewer, vcsrf) = login(&h, "bert").await;
        let body = |csrf: &str| format!("csrf_token={csrf}&wa_{}=1", channels[0]);
        assert_eq!(
            send(&h.app, post(&path, Some(&viewer), &body(&vcsrf)))
                .await
                .status,
            StatusCode::FORBIDDEN
        );
        let (admin, _) = login(&h, "anna").await;
        assert_eq!(
            send(&h.app, post(&path, Some(&admin), &body("wrong")))
                .await
                .status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(&h.app, post(&path, None, &body("x"))).await.status,
            StatusCode::SEE_OTHER
        );
        let stored = get_sensor(&h.state.pool, sensor).await.unwrap().unwrap();
        assert!(stored.channels.iter().all(|c| c.thresholds.is_empty()));
    }

    #[tokio::test]
    async fn status_channels_get_no_threshold_inputs() {
        let h = harness(true).await;
        let profile = add_v2c_profile(&h, "P", "public").await;
        let device = save_device(
            &h.state.pool,
            None,
            DeviceInput {
                kind: Default::default(),
                name: "AP".to_owned(),
                host: "192.0.2.2".to_owned(),
                group_id: None,
                profile_id: Some(profile),
                polling: Polling::default(),
            },
        )
        .await
        .unwrap();
        let sensor = create_sensor(
            &h.state.pool,
            device,
            "Port",
            &SensorSpec::InterfaceStatus { if_index: 1 },
        )
        .await
        .unwrap();
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(
            &h.app,
            get(&format!("/sensors/{sensor}/edit"), Some(&admin)),
        )
        .await;
        assert!(page.status == StatusCode::OK && !page.body.contains("name=\"wa_"));
        let reply = send(
            &h.app,
            post(
                &format!("/sensors/{sensor}/edit"),
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(
            reply.status,
            StatusCode::SEE_OTHER,
            "an empty form is a valid no-op"
        );
    }

    #[tokio::test]
    async fn chart_assets_are_served_from_the_binary() {
        let h = harness(true).await;
        for (path, content) in [
            ("/static/vendor/uplot/uPlot.iife.min.js", "javascript"),
            ("/static/vendor/uplot/uPlot.min.css", "css"),
            ("/static/chart.js", "javascript"),
            ("/static/time.js", "javascript"),
            ("/static/format.js", "javascript"),
            ("/static/samples.js", "javascript"),
            ("/static/pager.js", "javascript"),
            ("/static/events.js", "javascript"),
            ("/static/vendor/uplot/LICENSE", ""),
        ] {
            let reply = send(&h.app, get(path, None)).await;
            assert_eq!(reply.status, StatusCode::OK, "{path}");
            assert!(
                reply.headers[header::CONTENT_TYPE]
                    .to_str()
                    .unwrap()
                    .contains(content),
                "{path}"
            );
            assert!(!reply.body.is_empty());
        }
    }

    #[tokio::test]
    async fn admins_give_a_sensor_its_own_interval() {
        let h = harness(true).await;
        let (sensor, _) = traffic_sensor(&h).await;
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&admin))).await;
        assert!(page.body.contains("· alle 60 s"), "{}", page.body);
        let page = send(
            &h.app,
            get(&format!("/sensors/{sensor}/edit"), Some(&admin)),
        )
        .await;
        assert!(
            page.body.contains("name=\"interval\"") && page.body.contains("placeholder=\"60\"")
        );

        let path = format!("/sensors/{sensor}/edit");
        let reply = send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!("csrf_token={csrf}&interval=10"),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        assert!(reply.location().ends_with("msg=sensor_settings_saved"));
        let stored = get_sensor(&h.state.pool, sensor).await.unwrap().unwrap();
        assert_eq!(stored.interval_secs, Some(10));
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&admin))).await;
        assert!(page.body.contains("· alle 10 s"));
        let page = send(
            &h.app,
            get(&format!("/sensors/{sensor}/edit"), Some(&admin)),
        )
        .await;
        assert!(page.body.contains("eigenes Intervall"));
        let list = send(
            &h.app,
            get(
                &format!("/devices/{}/sensors", stored.device_id),
                Some(&admin),
            ),
        )
        .await;
        assert!(
            list.body.contains(&format!("/sensors/{sensor}\""))
                && !list.body.contains("· alle 10 s"),
            "the list leaves the description to the sensor page"
        );

        // Not a number, or outside 10 to 3600 seconds: explained, the input is kept.
        for (input, message) in [
            ("zehn", "ganze Zahl von Sekunden"),
            ("5", "zwischen 10 und 3600 Sekunden"),
        ] {
            let page = send(
                &h.app,
                post(
                    &path,
                    Some(&admin),
                    &format!("csrf_token={csrf}&interval={input}"),
                ),
            )
            .await;
            assert_eq!(page.status, StatusCode::UNPROCESSABLE_ENTITY);
            assert!(page.body.contains(message), "{input}: {}", page.body);
            assert!(page.body.contains(&format!("value=\"{input}\"")));
        }
        assert_eq!(
            get_sensor(&h.state.pool, sensor)
                .await
                .unwrap()
                .unwrap()
                .interval_secs,
            Some(10)
        );

        // An empty field returns to the device's interval; viewers may not change it.
        send(
            &h.app,
            post(&path, Some(&admin), &format!("csrf_token={csrf}&interval=")),
        )
        .await;
        assert_eq!(
            get_sensor(&h.state.pool, sensor)
                .await
                .unwrap()
                .unwrap()
                .interval_secs,
            None
        );
        let (viewer, viewer_csrf) = login(&h, "bert").await;
        let reply = send(
            &h.app,
            post(
                &path,
                Some(&viewer),
                &format!("csrf_token={viewer_csrf}&interval=10"),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admins_rename_sensors_and_errors_are_shown_on_the_page() {
        let h = harness(true).await;
        let (sensor, _) = traffic_sensor(&h).await;
        let device = get_sensor(&h.state.pool, sensor)
            .await
            .unwrap()
            .unwrap()
            .device_id;
        create_sensor(&h.state.pool, device, "Ping", &SensorSpec::ping())
            .await
            .unwrap();
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(
            &h.app,
            get(&format!("/sensors/{sensor}/edit"), Some(&admin)),
        )
        .await;
        assert!(
            page.body.contains("action=\"/sensors/1/edit\"")
                && page.body.contains("value=\"Traffic Port 1\"")
        );

        let path = format!("/sensors/{sensor}/edit");
        let reply = send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!("csrf_token={csrf}&name=Uplink+Keller"),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        assert!(reply.location().ends_with("msg=sensor_settings_saved"));
        let renamed = get_sensor(&h.state.pool, sensor).await.unwrap().unwrap();
        assert_eq!(
            (renamed.name.as_str(), renamed.channels.len()),
            ("Uplink Keller", 2)
        );

        let page = send(
            &h.app,
            post(&path, Some(&admin), &format!("csrf_token={csrf}&name=Ping")),
        )
        .await;
        assert_eq!(page.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(page.body.contains("existiert an diesem Gerät bereits"));
        assert!(
            page.body.contains("value=\"Ping\""),
            "the rejected input is kept"
        );
        let page = send(
            &h.app,
            post(&path, Some(&admin), &format!("csrf_token={csrf}&name=+")),
        )
        .await;
        assert_eq!(page.status, StatusCode::UNPROCESSABLE_ENTITY);

        let (viewer, vcsrf) = login(&h, "bert").await;
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&viewer))).await;
        assert!(!page.body.contains("/edit\""));
        assert_eq!(
            send(
                &h.app,
                post(&path, Some(&viewer), &format!("csrf_token={vcsrf}&name=X"))
            )
            .await
            .status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            get_sensor(&h.state.pool, sensor)
                .await
                .unwrap()
                .unwrap()
                .name,
            "Uplink Keller"
        );
    }

    async fn device(h: &Harness) -> i64 {
        let profile = add_v2c_profile(h, "Q", "public").await;
        save_device(
            &h.state.pool,
            None,
            DeviceInput {
                kind: Default::default(),
                name: "Switch Q".to_owned(),
                host: "192.0.2.9".to_owned(),
                group_id: None,
                profile_id: Some(profile),
                polling: Polling::default(),
            },
        )
        .await
        .unwrap()
    }

    const IF_OPER_1: &str = "1.3.6.1.2.1.2.2.1.8.1";

    #[tokio::test]
    async fn named_status_values_are_suggested_rated_and_reset() {
        let h = harness(true).await;
        let dev = device(&h).await;
        let spec = SensorSpec::Oid {
            oid: crate::sensors::spec::parse_oid(IF_OPER_1).unwrap(),
            mode: crate::sensors::snmp::OidMode::Gauge,
            scale: 1.0,
            unit: None,
        };
        let sensor = create_sensor(&h.state.pool, dev, "Port 1 roh", &spec)
            .await
            .unwrap();
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(
            &h.app,
            get(&format!("/sensors/{sensor}/edit"), Some(&admin)),
        )
        .await;
        for needle in [
            "Noch nicht bewertet",
            "<select name=\"st_1\"",
            "<option value=\"down\" selected>Ausfall</option>",
            "<option value=\"warning\" selected>Warnung</option>",
            "lowerLayerDown",
        ] {
            assert!(page.body.contains(needle), "{needle}: {}", page.body);
        }

        // Saving the page unchanged (its suggestions as shown) keeps the values unrated.
        let mut unchanged = format!("csrf_token={csrf}");
        for value in 1..=7 {
            let select = format!("<select name=\"st_{value}\"");
            let rest = &page.body[page.body.find(&select).unwrap()..];
            let rest = &rest[..rest.find("</select>").unwrap()];
            let chosen = rest
                .split("<option value=\"")
                .skip(1)
                .find(|o| o.contains("selected"))
                .unwrap();
            let rating = &chosen[..chosen.find('"').unwrap()];
            unchanged.push_str(&format!("&st_{value}={rating}"));
        }
        let edit = format!("/sensors/{sensor}/edit");
        let reply = send(&h.app, post(&edit, Some(&admin), &unchanged)).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        let stored = get_sensor(&h.state.pool, sensor).await.unwrap().unwrap();
        assert!(
            stored.channels[0].states.is_empty(),
            "suggestions are not stored"
        );

        let path = format!("/sensors/{sensor}/states");
        let mut form = format!("csrf_token={csrf}");
        for value in 1..=7 {
            let rating = match value {
                1 | 5 => "up",
                2 => "warning",
                _ => "down",
            };
            form.push_str(&format!("&st_{value}={rating}"));
        }
        let reply = send(&h.app, post(&edit, Some(&admin), &form)).await;
        assert_eq!(
            reply.location(),
            format!("/sensors/{sensor}?msg=sensor_settings_saved")
        );
        let stored = get_sensor(&h.state.pool, sensor).await.unwrap().unwrap();
        let states = &stored.channels[0].states;
        assert_eq!(states.len(), 7);
        assert_eq!(
            (states[1].label.as_str(), states[1].status),
            ("down", Status::Warning)
        );
        let page = send(&h.app, get(&edit, Some(&admin))).await;
        assert!(
            page.body.contains("bewertet</span>")
                && page.body.contains("Bewertung ausschalten")
                && page.body.contains("form=\"states-reset\"")
        );

        let incomplete = send(
            &h.app,
            post(&edit, Some(&admin), &format!("csrf_token={csrf}&st_1=up")),
        )
        .await;
        assert_eq!(incomplete.status, StatusCode::UNPROCESSABLE_ENTITY);

        let (viewer, vcsrf) = login(&h, "bert").await;
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&viewer))).await;
        assert!(page.body.contains("down (2): Warnung") && !page.body.contains("/states"));
        assert_eq!(
            send(
                &h.app,
                post(
                    &path,
                    Some(&viewer),
                    &format!("csrf_token={vcsrf}&action=reset")
                )
            )
            .await
            .status,
            StatusCode::FORBIDDEN
        );

        let reply = send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!("csrf_token={csrf}&action=reset"),
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/sensors/{sensor}?msg=states_reset")
        );
        let stored = get_sensor(&h.state.pool, sensor).await.unwrap().unwrap();
        assert!(stored.channels[0].states.is_empty());

        // Sensors without named values have no ratings.
        let ping = create_sensor(&h.state.pool, dev, "Ping", &SensorSpec::ping())
            .await
            .unwrap();
        let page = send(&h.app, get(&format!("/sensors/{ping}/edit"), Some(&admin))).await;
        assert!(!page.body.contains("Statuswerte"));
        let reply = send(
            &h.app,
            post(
                &format!("/sensors/{ping}/states"),
                Some(&admin),
                &format!("csrf_token={csrf}&action=reset"),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn the_interface_status_default_can_be_softened() {
        let h = harness(true).await;
        let dev = device(&h).await;
        let sensor = create_sensor(
            &h.state.pool,
            dev,
            "Status Port 1",
            &SensorSpec::InterfaceStatus { if_index: 1 },
        )
        .await
        .unwrap();
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(
            &h.app,
            get(&format!("/sensors/{sensor}/edit"), Some(&admin)),
        )
        .await;
        assert!(page.body.contains("Standard: nur „up“ ist in Ordnung"));
        assert!(page.body.contains("<select name=\"st_2\" aria-label=\"Bewertung für down\">\n              <option value=\"up\">In Ordnung</option>\n              <option value=\"warning\">Warnung</option>\n              <option value=\"down\" selected>"));
        let form = format!(
            "csrf_token={csrf}&st_1=up&st_2=warning&st_3=down&st_4=down&st_5=down&st_6=down&st_7=down"
        );
        send(
            &h.app,
            post(&format!("/sensors/{sensor}/edit"), Some(&admin), &form),
        )
        .await;
        let page = send(
            &h.app,
            get(&format!("/sensors/{sensor}/edit"), Some(&admin)),
        )
        .await;
        assert!(page.body.contains("Standard wiederherstellen"));
    }

    #[tokio::test]
    async fn the_page_and_its_chart_data_are_shown_in_english() {
        let h = harness(true).await;
        let (sensor, channels) = traffic_sensor(&h).await;
        crate::storage::users::set_language(&h.state.pool, 1, Some(Lang::En))
            .await
            .unwrap();
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&admin))).await;
        for text in [
            "<html lang=\"en\">",
            "Inbound",
            "12.5 Mbit/s",
            "first poll: OK",
            "\"decimal\":\".\"",
            "\"samples-sum\":\"Sum\"",
        ] {
            assert!(page.body.contains(text), "{text} missing");
        }
        let data = send(
            &h.app,
            get(&format!("/sensors/{sensor}/data?range=1h"), Some(&admin)),
        )
        .await;
        assert!(data.body.contains("\"label\":\"Inbound\""), "{}", data.body);
        let settings = send(
            &h.app,
            get(&format!("/sensors/{sensor}/edit"), Some(&admin)),
        )
        .await;
        for text in ["Warning above", "General", "Settings"] {
            assert!(settings.body.contains(text), "{text} missing");
        }
        let form = format!("csrf_token={csrf}&wa_{}=abc", channels[0]);
        let refused = send(
            &h.app,
            post(&format!("/sensors/{sensor}/edit"), Some(&admin), &form),
        )
        .await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(refused.body.contains("A threshold must be a number."));
    }
}
