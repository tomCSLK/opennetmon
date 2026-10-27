//! Sensor detail page: current values, chart (uPlot, data from a JSON endpoint), thresholds, events.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::{Form, Json};
use serde::{Deserialize, Serialize};

use crate::audit::{Action, Changes, Kind};
use crate::devices::Device;
use crate::error::{Error, Result};
use crate::i18n::{Lang, Msg};
use crate::sensors::Unit;
use crate::sensors::format::{channel_label, input_unit, parse_threshold, threshold_input};
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

const EVENT_LIMIT: u32 = 20;
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

struct ChannelView {
    id: i64,
    label: String,
    latest: String,
    latest_at: String,
    /// Threshold inputs: warn above, warn below, down above, down below (in `input_unit`).
    fields: [String; 4],
    unit_label: String,
    has_thresholds: bool,
}

struct StateRow {
    value: i64,
    label: String,
    /// `up`, `warning` or `down`.
    status: &'static str,
}

/// The rating form for sensors whose values have names (MIB enumeration).
struct StatesView {
    rows: Vec<StateRow>,
    /// Ratings are stored; otherwise the rows show a suggestion (or the interface default).
    stored: bool,
    interface: bool,
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
    channels: Vec<ChannelView>,
    ranges: Vec<RangeLink>,
    /// How far back the table can reach in this range; no choice when there is only one.
    spans: Vec<SpanOption>,
    range: &'static str,
    events: Vec<EventView>,
    flash: Option<String>,
    error: Option<String>,
    /// Name field of the rename form (the rejected input after an error).
    new_name: String,
    rename_failed: bool,
    /// The interval the sensor is polled at (its own or the device's).
    interval: u32,
    /// Interval field of its form: the own interval, empty for the device's, or the rejected input.
    interval_field: String,
    interval_failed: bool,
    states: Option<StatesView>,
    /// Availability for 24 h, 7 and 30 days: (label, value, downtime).
    availability: Vec<(String, String, String)>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

async fn load(state: &AppState, id: i64) -> Result<(SensorRecord, Device)> {
    let sensor = store::get_sensor(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)?;
    let device = devices::get_device(&state.pool, sensor.device_id)
        .await?
        .ok_or(Error::NotFound)?;
    Ok((sensor, device))
}

fn fields_of(lang: Lang, t: &Thresholds, unit: Unit) -> [String; 4] {
    [t.warn_above, t.warn_below, t.down_above, t.down_below].map(|v| threshold_input(lang, v, unit))
}

/// Variable parts of the page.
struct View<'a> {
    range: Range,
    flash: Option<String>,
    error: Option<String>,
    /// Rejected threshold input to echo back (by field name).
    submitted: Option<&'a HashMap<String, String>>,
    /// Rejected new name to echo back.
    rejected_name: Option<String>,
    /// Rejected interval input to echo back.
    rejected_interval: Option<String>,
}

async fn render(
    state: &AppState,
    user: &CurrentUser,
    sensor: SensorRecord,
    device: Device,
    view: View<'_>,
) -> Result<String> {
    let View {
        range,
        flash,
        error,
        submitted,
        rejected_name,
        rejected_interval,
    } = view;
    let latest = latest_values(&state.pool, &sensor.channels).await?;
    let mibs = state.mib_index();
    let value_format = ValueFormat::for_sensor(&sensor.spec, &mibs);
    let states = states_view(&sensor, &value_format);
    let channels = sensor
        .channels
        .iter()
        .map(|c| {
            let fields = match submitted {
                Some(form) => ["wa", "wb", "da", "db"].map(|p| {
                    form.get(&format!("{p}_{}", c.id))
                        .cloned()
                        .unwrap_or_default()
                }),
                None => fields_of(user.lang, &c.thresholds, c.unit),
            };
            ChannelView {
                id: c.id,
                label: user.lang.msg(&channel_label(&c.key)),
                latest: latest.get(&c.id).map_or_else(
                    || "–".to_owned(),
                    |(_, v)| value_format.format(user.lang, c.unit, *v),
                ),
                latest_at: latest
                    .get(&c.id)
                    .map_or_else(String::new, |(ts, _)| time_html(*ts)),
                fields,
                unit_label: match (&value_format.unit_label, c.unit) {
                    (Some(label), Unit::PerSecond) => format!("{label}/s"),
                    (Some(label), Unit::Count) => label.clone(),
                    _ => input_unit(c.unit).to_owned(),
                },
                has_thresholds: c.unit != Unit::State,
            }
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
    let events = status_store::events_of_sensor(&state.pool, sensor.id, EVENT_LIMIT)
        .await?
        .into_iter()
        .map(|e| EventView {
            at: time_html(e.ts_ms),
            change: match e.from {
                Some(from) => format!(
                    "{} → {}",
                    user.lang.get(from.label_id()),
                    user.lang.get(e.to.label_id())
                ),
                None => {
                    user.lang
                        .get1("event-first-poll", "status", user.lang.get(e.to.label_id()))
                }
            },
            message: crate::status::shown(user.lang, &e.message),
        })
        .collect();
    let availability = crate::web::reports::sensor_summary(state, user.lang, sensor.id).await?;
    let page = DetailTemplate {
        availability,
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
        channels,
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
        flash,
        error,
        rename_failed: rejected_name.is_some(),
        interval_failed: rejected_interval.is_some(),
        interval: sensor.interval(device.polling.interval_secs),
        interval_field: rejected_interval.unwrap_or_else(|| {
            sensor
                .interval_secs
                .map_or_else(String::new, |s| s.to_string())
        }),
        states,
        new_name: rejected_name.unwrap_or_else(|| sensor.name.clone()),
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
fn rated_channel(sensor: &SensorRecord) -> Option<&ChannelRecord> {
    match sensor.spec {
        SensorSpec::InterfaceStatus { .. } => sensor.channels.first(),
        SensorSpec::Oid { .. } => sensor.channels.first().filter(|c| c.unit == Unit::Count),
        _ => None,
    }
}

/// Named values: the MIB enumeration plus stored ratings of values the MIB does not (or no longer) list.
fn named_values(channel: &ChannelRecord, format: &ValueFormat) -> Vec<(i64, String)> {
    let mut values = format.labels.clone();
    for s in &channel.states {
        if !values.iter().any(|(v, _)| *v == s.value) {
            values.push((s.value, s.label.clone()));
        }
    }
    values.sort_by_key(|(v, _)| *v);
    values
}

fn states_view(sensor: &SensorRecord, format: &ValueFormat) -> Option<StatesView> {
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
            View {
                range: query.range(),
                flash: flash_message(user.lang, query.msg.as_deref()),
                error: None,
                submitted: None,
                rejected_name: None,
                rejected_interval: None,
            },
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
    values: Vec<Option<f64>>,
}

#[derive(Serialize)]
pub struct ChartJson {
    range: &'static str,
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
    let chart = chart_data(&state.pool, &sensor.channels, range, now_ms()).await?;
    Ok(Json(ChartJson {
        range: range.as_str(),
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
fn parse_limits(
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

pub async fn save_thresholds(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(form.get("csrf_token").map_or("", String::as_str))?;
    let (sensor, device) = load(&state, id).await?;
    match parse_limits(&sensor.channels, &form) {
        Ok(limits) => {
            store::set_thresholds(&state.pool, id, &limits).await?;
            tracing::info!(user = %user.username, sensor = id, "thresholds saved");
            let mut changes = Changes::new();
            for (channel, new) in &limits {
                if let Some(c) = sensor.channels.iter().find(|c| c.id == *channel) {
                    changes = changes.limits(channel_label(&c.key), c.unit, &c.thresholds, new);
                }
            }
            audit_changed(&state, &user, &sensor, &sensor.name, changes).await;
            Ok(Redirect::to(&format!("/sensors/{id}?msg=thresholds_saved")).into_response())
        }
        Err(Error::Invalid(message)) => {
            let html = render(
                &state,
                &user,
                sensor,
                device,
                View {
                    range: Range::Day,
                    flash: None,
                    error: Some(user.lang.msg(&message)),
                    submitted: Some(&form),
                    rejected_name: None,
                    rejected_interval: None,
                },
            )
            .await?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response())
        }
        Err(e) => Err(e),
    }
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
    let rating = |list: &[ValueState], value: i64| {
        list.iter()
            .find(|s| s.value == value)
            .map_or("audit-not-rated", |s| s.status.label_id())
    };
    let mut seen = std::collections::HashSet::new();
    let mut changes = Changes::new();
    for value in channel.states.iter().chain(&states) {
        if !seen.insert(value.value) {
            continue;
        }
        let (old, new) = (
            rating(&channel.states, value.value),
            rating(&states, value.value),
        );
        if old != new {
            changes = changes.note(
                Msg::new("audit-change")
                    .with("field", value.label.as_str())
                    .with("old", Msg::new(old))
                    .with("new", Msg::new(new)),
            );
        }
    }
    audit_changed(&state, &user, &sensor, &sensor.name, changes).await;
    let msg = if states.is_empty() {
        "states_reset"
    } else {
        "states_saved"
    };
    Ok(Redirect::to(&format!("/sensors/{id}?msg={msg}")).into_response())
}

#[derive(Deserialize)]
pub struct RenameForm {
    csrf_token: String,
    name: String,
}

pub async fn rename(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<RenameForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let (sensor, device) = load(&state, id).await?;
    match store::rename_sensor(&state.pool, id, &form.name).await {
        Ok(()) => {
            tracing::info!(user = %user.username, sensor = id, "sensor renamed");
            let new = form.name.trim();
            let changes = Changes::new().field("audit-field-name", &sensor.name, new);
            audit_changed(&state, &user, &sensor, new, changes).await;
            Ok(Redirect::to(&format!("/sensors/{id}?msg=sensor_renamed")).into_response())
        }
        Err(Error::Invalid(message) | Error::Conflict(message)) => {
            let html = render(
                &state,
                &user,
                sensor,
                device,
                View {
                    range: Range::Day,
                    flash: None,
                    error: Some(user.lang.msg(&message)),
                    submitted: None,
                    rejected_name: Some(form.name),
                    rejected_interval: None,
                },
            )
            .await?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response())
        }
        Err(e) => Err(e),
    }
}

#[derive(Deserialize)]
pub struct IntervalForm {
    csrf_token: String,
    #[serde(default)]
    interval: String,
}

/// Sets the sensor's own polling interval; an empty field returns to the device's.
pub async fn set_interval(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<IntervalForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let (sensor, device) = load(&state, id).await?;
    let input = form.interval.trim();
    let saved = match input {
        "" => Ok(None),
        text => text
            .parse::<u32>()
            .map(Some)
            .map_err(|_| Error::invalid(Msg::new("detail-interval-invalid"))),
    };
    let saved = match saved {
        Ok(secs) => store::set_interval(&state.pool, id, secs, &device.polling)
            .await
            .map(|()| secs),
        Err(e) => Err(e),
    };
    match saved {
        Ok(secs) => {
            tracing::info!(user = %user.username, sensor = id, interval = ?secs, "sensor interval set");
            let shown = |s: Option<u32>| s.map_or_else(String::new, |s| s.to_string());
            let changes = Changes::new().field(
                "audit-field-interval",
                shown(sensor.interval_secs),
                shown(secs),
            );
            audit_changed(&state, &user, &sensor, &sensor.name, changes).await;
            Ok(Redirect::to(&format!("/sensors/{id}?msg=interval_saved")).into_response())
        }
        Err(Error::Invalid(message)) => {
            let html = render(
                &state,
                &user,
                sensor,
                device,
                View {
                    range: Range::Day,
                    flash: None,
                    error: Some(user.lang.msg(&message)),
                    submitted: None,
                    rejected_name: None,
                    rejected_interval: Some(input.to_owned()),
                },
            )
            .await?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response())
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{DeviceInput, Polling};
    use crate::sensors::spec::SensorSpec;
    use crate::status::Assessment;
    use crate::storage::devices::save_device;
    use crate::storage::sensors::{Sample, create_sensor, get_sensor, insert_samples};
    use crate::storage::status::record_poll;
    use crate::web::testutil::*;
    use axum::http::header;

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
            "/static/samples.js",
            "id=\"samples\"",
            "data-src=\"/sensors/1/samples?range=24h\"",
            "id=\"samples-span\" data-range=\"24h\"",
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
            !page.body.contains("name=\"csrf_token\"") || !page.body.contains("/thresholds"),
            "viewers get no form"
        );
        assert!(page.body.contains("nur Administratoren"));

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
    async fn admins_save_thresholds_in_the_display_unit() {
        let h = harness(true).await;
        let (sensor, channels) = traffic_sensor(&h).await;
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&admin))).await;
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
            post(
                &format!("/sensors/{sensor}/thresholds"),
                Some(&admin),
                &body,
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/sensors/{sensor}?msg=thresholds_saved")
        );
        let stored = get_sensor(&h.state.pool, sensor).await.unwrap().unwrap();
        assert_eq!(stored.channels[0].thresholds.warn_above, Some(80_500_000.0));
        assert_eq!(stored.channels[0].thresholds.down_above, Some(95_000_000.0));
        assert!(stored.channels[1].thresholds.is_empty());

        let page = send(
            &h.app,
            get(
                &format!("/sensors/{sensor}?msg=thresholds_saved"),
                Some(&admin),
            ),
        )
        .await;
        assert!(
            page.body.contains("Schwellwerte gespeichert.") && page.body.contains("value=\"80,5\"")
        );
    }

    #[tokio::test]
    async fn bad_thresholds_are_explained_and_nothing_is_saved() {
        let h = harness(true).await;
        let (sensor, channels) = traffic_sensor(&h).await;
        let (admin, csrf) = login(&h, "anna").await;
        let path = format!("/sensors/{sensor}/thresholds");
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
        let path = format!("/sensors/{sensor}/thresholds");
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
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&admin))).await;
        assert!(!page.body.contains("name=\"wa_"));
        let reply = send(
            &h.app,
            post(
                &format!("/sensors/{sensor}/thresholds"),
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
        assert!(page.body.contains("action=\"/sensors/1/interval\""));
        assert!(page.body.contains("placeholder=\"60\""));

        let path = format!("/sensors/{sensor}/interval");
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
        assert!(reply.location().ends_with("msg=interval_saved"));
        let stored = get_sensor(&h.state.pool, sensor).await.unwrap().unwrap();
        assert_eq!(stored.interval_secs, Some(10));
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&admin))).await;
        assert!(page.body.contains("· alle 10 s") && page.body.contains("eigenes Intervall"));
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
        create_sensor(&h.state.pool, device, "Ping", &SensorSpec::Ping)
            .await
            .unwrap();
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&admin))).await;
        assert!(
            page.body.contains("action=\"/sensors/1/rename\"")
                && page.body.contains("value=\"Traffic Port 1\"")
        );

        let path = format!("/sensors/{sensor}/rename");
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
        assert!(reply.location().ends_with("msg=sensor_renamed"));
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
        assert!(!page.body.contains("/rename"));
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
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&admin))).await;
        for needle in [
            "Noch nicht bewertet",
            "<select name=\"st_1\"",
            "<option value=\"down\" selected>Ausfall</option>",
            "<option value=\"warning\" selected>Warnung</option>",
            "lowerLayerDown",
        ] {
            assert!(page.body.contains(needle), "{needle}: {}", page.body);
        }

        let path = format!("/sensors/{sensor}/states");
        let mut form = format!("csrf_token={csrf}&action=save");
        for value in 1..=7 {
            let rating = match value {
                1 | 5 => "up",
                2 => "warning",
                _ => "down",
            };
            form.push_str(&format!("&st_{value}={rating}"));
        }
        let reply = send(&h.app, post(&path, Some(&admin), &form)).await;
        assert_eq!(
            reply.location(),
            format!("/sensors/{sensor}?msg=states_saved")
        );
        let stored = get_sensor(&h.state.pool, sensor).await.unwrap().unwrap();
        let states = &stored.channels[0].states;
        assert_eq!(states.len(), 7);
        assert_eq!(
            (states[1].label.as_str(), states[1].status),
            ("down", Status::Warning)
        );
        let page = send(&h.app, get(reply.location(), Some(&admin))).await;
        assert!(
            page.body.contains("bewertet</span>") && page.body.contains("Bewertung ausschalten")
        );

        let incomplete = send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!("csrf_token={csrf}&action=save&st_1=up"),
            ),
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
        let ping = create_sensor(&h.state.pool, dev, "Ping", &SensorSpec::Ping)
            .await
            .unwrap();
        let page = send(&h.app, get(&format!("/sensors/{ping}"), Some(&admin))).await;
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
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&admin))).await;
        assert!(page.body.contains("Standard: nur „up“ ist in Ordnung"));
        assert!(page.body.contains("<select name=\"st_2\" aria-label=\"Bewertung für down\" class=\"compact\">\n              <option value=\"up\">In Ordnung</option>\n              <option value=\"warning\">Warnung</option>\n              <option value=\"down\" selected>"));
        let form = format!(
            "csrf_token={csrf}&action=save&st_1=up&st_2=warning&st_3=down&st_4=down&st_5=down&st_6=down&st_7=down"
        );
        send(
            &h.app,
            post(&format!("/sensors/{sensor}/states"), Some(&admin), &form),
        )
        .await;
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&admin))).await;
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
            "Warning above",
            "Save thresholds",
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
        let form = format!("csrf_token={csrf}&wa_{}=abc", channels[0]);
        let refused = send(
            &h.app,
            post(
                &format!("/sensors/{sensor}/thresholds"),
                Some(&admin),
                &form,
            ),
        )
        .await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(refused.body.contains("A threshold must be a number."));
    }
}
