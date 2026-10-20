//! Sensor pages: the sensors of a device, adding sensors (also from interface discovery),
//! pausing and deleting. Everyone logged in may look; changes are admin only.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use askama::Template;
use axum::Form;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::devices::Device;
use crate::discovery::discover_interfaces;
use crate::error::{Error, Result};
use crate::i18n::{Lang, Msg};
use crate::mib::MibIndex;
use crate::sensors::Unit;
use crate::sensors::format::channel_label;
use crate::sensors::snmp::OidMode;
use crate::sensors::spec::SensorSpec;
use crate::snmp::{SnmpClient, SnmpTarget};
use crate::status::{Status, device_status};
use crate::storage::measurements::{bucket_averages, latest_values};
use crate::storage::sensors::{self as store, SensorRecord};
use crate::storage::{devices, status as status_store};
use crate::web::auth::{CsrfForm, CurrentUser};
use crate::web::overview::FlashQuery;
use crate::web::sensor_text::ValueFormat;
use crate::web::sparkline::{self, Trend};
use crate::web::{AppState, Crumb, Layout, flash_message, group_trail};

/// Discovery never blocks a request for long, whatever the device is configured to.
const DISCOVERY_MAX_TIMEOUT_SECS: u32 = 10;
const DISCOVERY_MAX_RETRIES: u32 = 1;
/// Upper bound for one batch of interface sensors (a form with more fields is not a real form).
const MAX_FORM_FIELDS: usize = 2_000;
const MAX_NAME_CHARS: usize = 64;

struct SensorRowView {
    id: i64,
    name: String,
    description: String,
    status_label: String,
    status_class: &'static str,
    message: String,
    /// Latest value per channel (traffic: in, out); empty when there is no recent one.
    values: Vec<String>,
    /// The values with their channel names, as a tooltip.
    value_title: String,
    /// The last hour (only for active sensors).
    trend: Trend,
}

#[derive(Clone)]
struct SensorFormValues {
    kind: String,
    name: String,
    oid: String,
    mode: String,
    scale: String,
    unit: String,
}

impl Default for SensorFormValues {
    fn default() -> Self {
        Self {
            kind: "ping".to_owned(),
            name: String::new(),
            oid: String::new(),
            mode: "auto".to_owned(),
            scale: "1".to_owned(),
            unit: String::new(),
        }
    }
}

#[derive(Template)]
#[template(path = "sensors.html")]
struct SensorsTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    device: Device,
    /// Groups above the device, for the breadcrumbs.
    trail: Vec<Crumb>,
    device_status_label: String,
    device_status_class: &'static str,
    /// Active sensors (polled).
    rows: Vec<SensorRowView>,
    /// Inactive sensors (not polled until activated).
    inactive: Vec<SensorRowView>,
    flash: Option<String>,
    error: Option<String>,
    form: SensorFormValues,
    /// The device template this device was set up with.
    template_name: Option<String>,
}

struct DiscoverRowView {
    index: u32,
    name: String,
    descr: String,
    alias: String,
    status: &'static str,
    speed: String,
    preselect: bool,
    has_traffic: bool,
    has_status: bool,
}

#[derive(Template)]
#[template(path = "sensor_discover.html")]
struct DiscoverTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    device: Device,
    /// Groups above the device, for the breadcrumbs.
    trail: Vec<Crumb>,
    rows: Vec<DiscoverRowView>,
    failure: Option<String>,
}

#[derive(Deserialize)]
pub struct SensorForm {
    csrf_token: String,
    kind: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    oid: String,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    scale: String,
    #[serde(default)]
    unit: String,
}

impl SensorForm {
    fn values(&self) -> SensorFormValues {
        SensorFormValues {
            kind: self.kind.clone(),
            name: self.name.clone(),
            oid: self.oid.clone(),
            mode: self.mode.clone(),
            scale: self.scale.clone(),
            unit: self.unit.clone(),
        }
    }

    /// Name and configuration for the chosen sensor type. A custom OID may be given by MIB name;
    /// mode ("auto"), unit and name then default to what the MIB says.
    fn to_spec(&self, mibs: &MibIndex) -> Result<(String, SensorSpec)> {
        let invalid = |id: &'static str| Error::invalid(Msg::new(id));
        let (default_name, spec) = match self.kind.as_str() {
            "ping" => ("Ping".to_owned(), SensorSpec::Ping),
            "uptime" => ("Uptime".to_owned(), SensorSpec::Uptime),
            "oid" => {
                let target = mibs.oid_target(&self.oid).map_err(Error::invalid)?;
                let object = target.object.as_ref();
                let mode = match self.mode.as_str() {
                    "gauge" => OidMode::Gauge,
                    "counter" => OidMode::Counter,
                    "auto" | "" => {
                        if object.and_then(|o| o.base).is_some_and(|b| b.is_counter()) {
                            OidMode::Counter
                        } else {
                            OidMode::Gauge
                        }
                    }
                    _ => {
                        return Err(invalid("sensors-mode-missing"));
                    }
                };
                let scale = match self.scale.trim() {
                    "" => 1.0,
                    text => text
                        .replace(',', ".")
                        .parse::<f64>()
                        .map_err(|_| invalid("sensors-scale-not-number"))?,
                };
                let unit = match self.unit.trim() {
                    "" => object.and_then(|o| o.units.clone()),
                    text => Some(text.to_owned()),
                };
                let default_name = object
                    .and_then(|_| mibs.name_of(&target.oid))
                    .unwrap_or_default();
                (
                    default_name,
                    SensorSpec::Oid {
                        oid: target.oid,
                        mode,
                        scale,
                        unit,
                    },
                )
            }
            _ => return Err(invalid("sensors-type-missing")),
        };
        let name = match self.name.trim() {
            "" => default_name,
            name => name.to_owned(),
        };
        if name.is_empty() {
            return Err(invalid("sensors-name-missing"));
        }
        let name: String = name.chars().take(crate::devices::MAX_NAME_LEN).collect();
        Ok((name, spec))
    }
}

async fn load_device(state: &AppState, id: i64) -> Result<Device> {
    devices::get_device(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The last hour of an active sensor and its latest values (shown, and with channel names as
/// a tooltip). Status values and uptime show their status over time, measured values a line per
/// channel.
async fn recent(
    pool: &sqlx::SqlitePool,
    sensor: &SensorRecord,
    device: &Device,
    mibs: &MibIndex,
    t: Lang,
    now: i64,
) -> Result<(Trend, Vec<String>, String)> {
    let as_band = matches!(
        sensor.spec,
        SensorSpec::InterfaceStatus { .. } | SensorSpec::Uptime
    ) || sensor.channels.iter().any(|c| c.unit == Unit::State);
    let trend = if as_band {
        let from = now - sparkline::WINDOW_MS;
        let (start, changes) = status_store::status_history(pool, sensor.id, from).await?;
        match sparkline::band(start, &changes, from, now) {
            band if band.is_empty() => Trend::Empty,
            band => Trend::Band(band),
        }
    } else {
        let (bucket, count) = sparkline::buckets(device.polling.interval_secs);
        let from = now - bucket * i64::try_from(count).unwrap_or(0);
        let mut series = Vec::new();
        for channel in &sensor.channels {
            series.push(bucket_averages(pool, channel.id, from, bucket, count).await?);
        }
        match sparkline::lines(&series) {
            lines if lines.is_empty() => Trend::Empty,
            lines => Trend::Lines(lines),
        }
    };
    // Only a recent value is shown: an old one (sensor failing for a while) would mislead.
    let fresh_from = now - 3_000 * i64::from(device.polling.interval_secs) - 60_000;
    let latest = latest_values(pool, &sensor.channels).await?;
    let format = ValueFormat::for_sensor(&sensor.spec, mibs);
    let (mut values, mut titled) = (Vec::new(), Vec::new());
    for channel in &sensor.channels {
        if let Some((ts, value)) = latest.get(&channel.id)
            && *ts >= fresh_from
        {
            let shown = format.format(t, channel.unit, *value);
            titled.push(format!("{}: {shown}", t.msg(&channel_label(&channel.key))));
            values.push(shown);
        }
    }
    Ok((trend, values, titled.join(", ")))
}

async fn render_sensors(
    state: &AppState,
    user: &CurrentUser,
    device: Device,
    flash: Option<String>,
    error: Option<String>,
    form: SensorFormValues,
) -> Result<String> {
    let mibs = state.mib_index();
    let records = store::list_sensors(&state.pool, device.id).await?;
    let states = status_store::list_states(&state.pool).await?;
    let overall = device_status(
        records
            .iter()
            .map(|r| (r.enabled, states.get(&r.id).map(|s| s.status))),
    );
    let pool = &state.pool;
    let now = now_ms();
    let mut rows = Vec::new();
    let mut inactive = Vec::new();
    for r in &records {
        let state = states.get(&r.id);
        let status = if r.enabled {
            state.map_or(Status::Unknown, |s| s.status)
        } else {
            Status::Paused
        };
        let (trend, values, value_title) = if r.enabled {
            recent(pool, r, &device, &mibs, user.lang, now).await?
        } else {
            (Trend::Empty, Vec::new(), String::new())
        };
        let row = SensorRowView {
            id: r.id,
            name: r.name.clone(),
            description: crate::web::sensor_text::describe(user.lang, &r.spec, &mibs),
            status_label: user.lang.get(status.label_id()),
            status_class: status.as_str(),
            message: state
                .map(|s| crate::status::shown(user.lang, &s.message))
                .unwrap_or_default(),
            values,
            value_title,
            trend,
        };
        if r.enabled {
            rows.push(row);
        } else {
            inactive.push(row);
        }
    }
    let template_name =
        match crate::storage::templates::template_of_device(&state.pool, device.id).await? {
            Some(id) => crate::storage::templates::get_template(&state.pool, id)
                .await?
                .map(|t| t.name),
            None => None,
        };
    let page = SensorsTemplate {
        template_name,
        layout: user.layout(),
        t: user.lang,
        trail: group_trail(&state.pool, device.group_id).await?,
        device,
        device_status_label: user.lang.get(overall.label_id()),
        device_status_class: overall.as_str(),
        rows,
        inactive,
        flash,
        error,
        form,
    };
    Ok(page.render()?)
}

pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<FlashQuery>,
) -> Result<Html<String>> {
    let device = load_device(&state, id).await?;
    Ok(Html(
        render_sensors(
            &state,
            &user,
            device,
            flash_message(user.lang, query.msg.as_deref()),
            None,
            SensorFormValues::default(),
        )
        .await?,
    ))
}

pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<SensorForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let device = load_device(&state, id).await?;
    let mibs = state.mib_index();
    let outcome = match form.to_spec(&mibs) {
        Ok((_, spec)) if spec.uses_snmp() && !device.has_snmp() => {
            Err(Error::invalid(Msg::new("device-without-snmp")))
        }
        Ok((name, spec)) => match store::create_sensor(&state.pool, id, &name, &spec).await {
            Ok(sensor) => {
                crate::web::sensor_text::rate_new_sensor(&state.pool, sensor, &spec, &mibs)
                    .await
                    .map(|()| sensor)
            }
            Err(e) => Err(e),
        },
        Err(e) => Err(e),
    };
    match outcome {
        Ok(sensor) => {
            tracing::info!(user = %user.username, device = id, sensor, "sensor created");
            Ok(Redirect::to(&format!("/devices/{id}/sensors?msg=sensor_created")).into_response())
        }
        Err(Error::Invalid(message) | Error::Conflict(message)) => {
            let html = render_sensors(
                &state,
                &user,
                device,
                None,
                Some(user.lang.msg(&message)),
                form.values(),
            )
            .await?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response())
        }
        Err(e) => Err(e),
    }
}

/// Walks the device's interface tables and lets the admin pick interfaces.
pub async fn discover(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Result<Html<String>> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let device = load_device(&state, id).await?;
    let creds = crate::web::snmp_credentials(&state, &device).await?;
    let target = SnmpTarget {
        host: device.host.clone(),
        port: device.polling.port,
        timeout: Duration::from_secs(u64::from(
            device.polling.timeout_secs.min(DISCOVERY_MAX_TIMEOUT_SECS),
        )),
        retries: device.polling.retries.min(DISCOVERY_MAX_RETRIES),
    };
    let outcome = async {
        let mut client = SnmpClient::connect(&target, &creds).await?;
        discover_interfaces(&mut client).await
    }
    .await;
    tracing::info!(user = %user.username, device = id, ok = outcome.is_ok(), "interface discovery");

    let existing = store::list_sensors(&state.pool, id).await?;
    let has = |wanted: fn(&SensorSpec) -> Option<u32>, index: u32| {
        existing.iter().any(|r| wanted(&r.spec) == Some(index))
    };
    let (rows, failure) = match outcome {
        Ok(found) => (
            found
                .into_iter()
                .map(|i| DiscoverRowView {
                    index: i.index,
                    speed: i
                        .speed_mbps
                        .map(|m| {
                            if m >= 1000 && m % 1000 == 0 {
                                format!("{} Gbit/s", m / 1000)
                            } else {
                                format!("{m} Mbit/s")
                            }
                        })
                        .unwrap_or_default(),
                    status: i.oper_status_label(),
                    preselect: i.suggested(),
                    has_traffic: has(traffic_index, i.index),
                    has_status: has(status_index, i.index),
                    name: i.name,
                    descr: i.descr,
                    alias: i.alias,
                })
                .collect(),
            None,
        ),
        Err(e) => (Vec::new(), Some(user.lang.msg(&e.user_message()))),
    };
    let page = DiscoverTemplate {
        layout: user.layout(),
        t: user.lang,
        trail: group_trail(&state.pool, device.group_id).await?,
        device,
        rows,
        failure,
    };
    Ok(Html(page.render()?))
}

fn traffic_index(spec: &SensorSpec) -> Option<u32> {
    match spec {
        SensorSpec::InterfaceTraffic { if_index } => Some(*if_index),
        _ => None,
    }
}

fn status_index(spec: &SensorSpec) -> Option<u32> {
    match spec {
        SensorSpec::InterfaceStatus { if_index } => Some(*if_index),
        _ => None,
    }
}

/// `"{prefix} {name}"`, cut to the maximum name length.
fn sensor_name(prefix: &str, name: &str) -> String {
    format!("{prefix} {name}")
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect()
}

/// Creates the selected interface sensors. Form fields: `t_<ifIndex>` (traffic), `s_<ifIndex>`
/// (status) and `n_<ifIndex>` (display name from the discovery page).
pub async fn create_interfaces(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(fields): Form<HashMap<String, String>>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(fields.get("csrf_token").map_or("", String::as_str))?;
    load_device(&state, id).await?;
    if fields.len() > MAX_FORM_FIELDS {
        return Err(Error::invalid(Msg::new("form-too-large")));
    }

    let existing = store::list_sensors(&state.pool, id).await?;
    let mut taken: HashSet<InterfaceKey> =
        existing.iter().filter_map(|r| key_of(&r.spec)).collect();
    let mut wanted: Vec<(u32, bool)> = fields
        .keys()
        .filter_map(|k| {
            let (prefix, index) = k.split_once('_')?;
            let index: u32 = index.parse().ok()?;
            match prefix {
                "t" => Some((index, true)),
                "s" => Some((index, false)),
                _ => None,
            }
        })
        .collect();
    wanted.sort_unstable();
    if wanted.is_empty() {
        return Err(Error::invalid(Msg::new("sensors-select-interface")));
    }

    let mut created = 0usize;
    for (index, traffic) in wanted {
        let (spec, prefix) = if traffic {
            (SensorSpec::InterfaceTraffic { if_index: index }, "Traffic")
        } else {
            (SensorSpec::InterfaceStatus { if_index: index }, "Status")
        };
        let key = key_of(&spec);
        if key.is_some_and(|k| taken.contains(&k)) {
            continue; // already monitored
        }
        let display = fields
            .get(&format!("n_{index}"))
            .map(|n| n.trim())
            .filter(|n| !n.is_empty())
            .map_or_else(|| format!("Interface {index}"), str::to_owned);
        let name = sensor_name(prefix, &display);
        let result = match store::create_sensor(&state.pool, id, &name, &spec).await {
            // Two interfaces with the same name: make the name unique with the index.
            Err(Error::Conflict(_)) => {
                let suffix = format!(" #{index}");
                let short: String = name
                    .chars()
                    .take(MAX_NAME_CHARS - suffix.chars().count())
                    .collect();
                store::create_sensor(&state.pool, id, &format!("{short}{suffix}"), &spec).await
            }
            other => other,
        };
        result?;
        if let Some(k) = key {
            taken.insert(k);
        }
        created += 1;
    }
    tracing::info!(user = %user.username, device = id, created, "interface sensors created");
    Ok(Redirect::to(&format!("/devices/{id}/sensors?msg=sensors_added")).into_response())
}

/// Identity of an interface sensor: (traffic?, ifIndex).
type InterfaceKey = (bool, u32);

fn key_of(spec: &SensorSpec) -> Option<InterfaceKey> {
    traffic_index(spec)
        .map(|i| (true, i))
        .or_else(|| status_index(spec).map(|i| (false, i)))
}

async fn find_sensor(state: &AppState, id: i64) -> Result<SensorRecord> {
    store::get_sensor(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)
}

/// Activates, deactivates or deletes the ticked sensors of a device (fields `s_<id>=on`).
pub async fn bulk(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(fields): Form<HashMap<String, String>>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(fields.get("csrf_token").map_or("", String::as_str))?;
    load_device(&state, id).await?;
    let ids: Vec<i64> = fields
        .iter()
        .filter(|(k, v)| k.starts_with("s_") && v.as_str() == "on")
        .filter_map(|(k, _)| k[2..].parse().ok())
        .take(MAX_FORM_FIELDS)
        .collect();
    let action = fields.get("action").map_or("", String::as_str);
    let all_inactive = action == "activate_all";
    let ids = if all_inactive {
        store::list_sensors(&state.pool, id)
            .await?
            .into_iter()
            .filter(|s| !s.enabled)
            .map(|s| s.id)
            .collect()
    } else {
        ids
    };
    if ids.is_empty() {
        return Err(Error::invalid(Msg::new("sensors-select-sensor")));
    }
    let (changed, msg) = match action {
        "activate" | "activate_all" => (
            store::set_enabled_many(&state.pool, id, &ids, true).await?,
            "sensors_activated",
        ),
        "deactivate" => (
            store::set_enabled_many(&state.pool, id, &ids, false).await?,
            "sensors_deactivated",
        ),
        "delete" => (
            store::delete_many(&state.pool, id, &ids).await?,
            "sensors_deleted",
        ),
        _ => return Err(Error::invalid(Msg::new("unknown-action"))),
    };
    tracing::info!(user = %user.username, device = id, action, changed, "bulk sensor change");
    Ok(Redirect::to(&format!("/devices/{id}/sensors?msg={msg}")).into_response())
}

pub async fn toggle(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let sensor = find_sensor(&state, id).await?;
    store::set_enabled(&state.pool, id, !sensor.enabled).await?;
    tracing::info!(user = %user.username, sensor = id, enabled = !sensor.enabled, "sensor toggled");
    let msg = if sensor.enabled {
        "sensor_paused"
    } else {
        "sensor_resumed"
    };
    Ok(Redirect::to(&format!("/devices/{}/sensors?msg={msg}", sensor.device_id)).into_response())
}

pub async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let sensor = find_sensor(&state, id).await?;
    store::delete_sensor(&state.pool, id).await?;
    tracing::info!(user = %user.username, sensor = id, "sensor deleted");
    Ok(Redirect::to(&format!(
        "/devices/{}/sensors?msg=sensor_deleted",
        sensor.device_id
    ))
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{DeviceInput, Polling};
    use crate::snmp::testagent::{FakeAgent, FakeValue};
    use crate::storage::devices::save_device;
    use crate::web::testutil::*;

    const IF_DESCR: &[u64] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 2];
    const IF_TYPE: &[u64] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 3];
    const IF_OPER: &[u64] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 8];

    /// A device whose SNMP port is `port` (the fake agent), in a fresh harness.
    async fn device_on(h: &Harness, port: u16) -> i64 {
        let profile = add_v2c_profile(h, "Fake", "public").await;
        save_device(
            &h.state.pool,
            None,
            DeviceInput {
                name: "Switch".to_owned(),
                host: "127.0.0.1".to_owned(),
                group_id: None,
                profile_id: Some(profile),
                polling: Polling {
                    port,
                    interval_secs: 60,
                    timeout_secs: 2,
                    retries: 0,
                },
            },
        )
        .await
        .unwrap()
    }

    async fn agent_with_ports() -> FakeAgent {
        let agent = FakeAgent::spawn("public", "x", 1).await;
        for (index, name, status) in [
            (49153u64, "gigabitEthernet 1/0/1", 1),
            (49154, "gigabitEthernet 1/0/2", 2),
        ] {
            agent.set(
                &[IF_DESCR, &[index]].concat(),
                FakeValue::Text(name.to_owned()),
            );
            agent.set(&[IF_TYPE, &[index]].concat(), FakeValue::Integer(6));
            agent.set(&[IF_OPER, &[index]].concat(), FakeValue::Integer(status));
        }
        agent
    }

    fn csrf_only(csrf: &str) -> String {
        format!("csrf_token={csrf}")
    }

    #[tokio::test]
    async fn everyone_may_look_but_only_admins_may_change() {
        let h = harness(true).await;
        let device = device_on(&h, 161).await;
        let (admin, acsrf) = login(&h, "anna").await;
        send(
            &h.app,
            post(
                &format!("/devices/{device}/sensors"),
                Some(&admin),
                &format!("csrf_token={acsrf}&kind=ping&name="),
            ),
        )
        .await;

        let path = format!("/devices/{device}/sensors");
        assert_eq!(
            send(&h.app, get(&path, None)).await.status,
            StatusCode::SEE_OTHER
        );
        let (viewer, vcsrf) = login(&h, "bert").await;
        let page = send(&h.app, get(&path, Some(&viewer))).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains("Ping"));
        assert!(!page.body.contains("Sensor hinzufügen") && !page.body.contains("/toggle"));

        for (target, body) in [
            (path.clone(), format!("csrf_token={vcsrf}&kind=ping")),
            (format!("/devices/{device}/discover"), csrf_only(&vcsrf)),
            (
                format!("/devices/{device}/sensors/interfaces"),
                format!("csrf_token={vcsrf}&t_1=on"),
            ),
            ("/sensors/1/toggle".to_owned(), csrf_only(&vcsrf)),
            ("/sensors/1/delete".to_owned(), csrf_only(&vcsrf)),
        ] {
            assert_eq!(
                send(&h.app, post(&target, Some(&viewer), &body))
                    .await
                    .status,
                StatusCode::FORBIDDEN,
                "{target}"
            );
        }
        assert_eq!(
            store::list_sensors(&h.state.pool, device)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn a_wrong_csrf_token_is_rejected() {
        let h = harness(true).await;
        let device = device_on(&h, 161).await;
        let (admin, _) = login(&h, "anna").await;
        for (target, body) in [
            (
                format!("/devices/{device}/sensors"),
                "csrf_token=bad&kind=ping".to_owned(),
            ),
            (
                format!("/devices/{device}/discover"),
                "csrf_token=bad".to_owned(),
            ),
            (
                format!("/devices/{device}/sensors/interfaces"),
                "csrf_token=bad&t_1=on".to_owned(),
            ),
        ] {
            assert_eq!(
                send(&h.app, post(&target, Some(&admin), &body))
                    .await
                    .status,
                StatusCode::FORBIDDEN,
                "{target}"
            );
        }
    }

    #[tokio::test]
    async fn admin_adds_simple_and_oid_sensors() {
        let h = harness(true).await;
        let device = device_on(&h, 161).await;
        let (admin, csrf) = login(&h, "anna").await;
        let path = format!("/devices/{device}/sensors");
        for body in [
            format!("csrf_token={csrf}&kind=ping&name="),
            format!("csrf_token={csrf}&kind=uptime&name=Laufzeit"),
            format!(
                "csrf_token={csrf}&kind=oid&name=Temperatur&oid=1.3.6.1.4.1.99.1.0&mode=gauge&scale=0%2C1"
            ),
        ] {
            let reply = send(&h.app, post(&path, Some(&admin), &body)).await;
            assert_eq!(
                reply.location(),
                format!("{path}?msg=sensor_created"),
                "{body}"
            );
        }
        let stored = store::list_sensors(&h.state.pool, device).await.unwrap();
        let names: Vec<_> = stored.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["Ping", "Laufzeit", "Temperatur"],
            "ping, uptime, then own values"
        );
        assert!(matches!(
            &stored[2].spec,
            SensorSpec::Oid { scale, .. } if (*scale - 0.1).abs() < 1e-12
        ));
        let page = send(
            &h.app,
            get(&format!("{path}?msg=sensor_created"), Some(&admin)),
        )
        .await;
        assert!(page.body.contains("Sensor angelegt."));
        assert!(page.body.contains("OID 1.3.6.1.4.1.99.1.0"));
        assert!(page.body.contains("Unbekannt"), "never polled yet");
    }

    #[tokio::test]
    async fn invalid_sensor_input_is_explained_and_echoed() {
        let h = harness(true).await;
        let device = device_on(&h, 161).await;
        let (admin, csrf) = login(&h, "anna").await;
        let path = format!("/devices/{device}/sensors");
        for (body, expected) in [
            (
                "kind=oid&name=X&oid=1.3.x&mode=gauge&scale=1",
                "OID ist ungültig",
            ),
            (
                "kind=oid&name=&oid=1.3.6.1.4.1.99999.1&mode=gauge&scale=1",
                "Namen",
            ),
            (
                "kind=oid&name=X&oid=nichtVorhanden.0&mode=gauge&scale=1",
                "in keiner geladenen MIB",
            ),
            ("kind=oid&name=X&oid=1.3.6.1&mode=gauge&scale=0", "Faktor"),
            ("kind=oid&name=X&oid=1.3.6.1&mode=gauge&scale=abc", "Faktor"),
            ("kind=oid&name=X&oid=1.3.6.1&mode=wrong&scale=1", "Modus"),
            ("kind=bogus&name=X", "Sensortyp"),
        ] {
            let reply = send(
                &h.app,
                post(&path, Some(&admin), &format!("csrf_token={csrf}&{body}")),
            )
            .await;
            assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
            assert!(reply.body.contains(expected), "{body}: {}", reply.body);
        }
        // Duplicate names are reported on the same page, with the input kept.
        send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!("csrf_token={csrf}&kind=ping&name=Ping"),
            ),
        )
        .await;
        let dup = send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!("csrf_token={csrf}&kind=uptime&name=ping"),
            ),
        )
        .await;
        assert_eq!(dup.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(dup.body.contains("existiert an diesem Gerät bereits"));
        assert_eq!(
            store::list_sensors(&h.state.pool, device)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn unknown_devices_and_sensors_are_404() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        assert_eq!(
            send(&h.app, get("/devices/99/sensors", Some(&admin)))
                .await
                .status,
            StatusCode::NOT_FOUND
        );
        for target in [
            "/devices/99/discover",
            "/sensors/99/toggle",
            "/sensors/99/delete",
        ] {
            assert_eq!(
                send(&h.app, post(target, Some(&admin), &csrf_only(&csrf)))
                    .await
                    .status,
                StatusCode::NOT_FOUND,
                "{target}"
            );
        }
    }

    #[tokio::test]
    async fn discovery_lists_interfaces_and_creates_the_selected_sensors() {
        let agent = agent_with_ports().await;
        let h = harness(true).await;
        let device = device_on(&h, agent.port).await;
        let (admin, csrf) = login(&h, "anna").await;

        let page = send(
            &h.app,
            post(
                &format!("/devices/{device}/discover"),
                Some(&admin),
                &csrf_only(&csrf),
            ),
        )
        .await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains("gigabitEthernet 1/0/1") && page.body.contains("49154"));
        // Only the port that is up is preselected for traffic.
        assert!(page.body.contains("name=\"t_49153\" value=\"on\" aria-label=\"Traffic-Sensor für gigabitEthernet 1/0/1\"\n              checked"));
        assert!(!page.body.contains("name=\"t_49154\" value=\"on\" aria-label=\"Traffic-Sensor für gigabitEthernet 1/0/2\"\n              checked"));

        let body = format!(
            "csrf_token={csrf}&t_49153=on&s_49153=on&n_49153=gigabitEthernet+1%2F0%2F1&t_49154=on&n_49154=gigabitEthernet+1%2F0%2F2"
        );
        let reply = send(
            &h.app,
            post(
                &format!("/devices/{device}/sensors/interfaces"),
                Some(&admin),
                &body,
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/devices/{device}/sensors?msg=sensors_added")
        );
        let stored = store::list_sensors(&h.state.pool, device).await.unwrap();
        let mut names: Vec<_> = stored.iter().map(|s| s.name.clone()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "Status gigabitEthernet 1/0/1",
                "Traffic gigabitEthernet 1/0/1",
                "Traffic gigabitEthernet 1/0/2"
            ]
        );
        assert!(
            stored
                .iter()
                .any(|s| s.spec == SensorSpec::InterfaceTraffic { if_index: 49154 })
        );

        // Existing sensors are locked in the next discovery and are never created twice.
        let again = send(
            &h.app,
            post(
                &format!("/devices/{device}/discover"),
                Some(&admin),
                &csrf_only(&csrf),
            ),
        )
        .await;
        assert!(again.body.contains("disabled checked"));
        send(
            &h.app,
            post(
                &format!("/devices/{device}/sensors/interfaces"),
                Some(&admin),
                &body,
            ),
        )
        .await;
        assert_eq!(
            store::list_sensors(&h.state.pool, device)
                .await
                .unwrap()
                .len(),
            3
        );
    }

    #[tokio::test]
    async fn interfaces_with_the_same_name_get_a_unique_sensor_name() {
        let h = harness(true).await;
        let device = device_on(&h, 161).await;
        let (admin, csrf) = login(&h, "anna").await;
        let body = format!("csrf_token={csrf}&t_1=on&n_1=Port&t_2=on&n_2=Port&t_3=on");
        send(
            &h.app,
            post(
                &format!("/devices/{device}/sensors/interfaces"),
                Some(&admin),
                &body,
            ),
        )
        .await;
        let mut names: Vec<_> = store::list_sensors(&h.state.pool, device)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["Traffic Interface 3", "Traffic Port", "Traffic Port #2"]
        );
    }

    #[tokio::test]
    async fn selecting_nothing_or_garbage_creates_nothing() {
        let h = harness(true).await;
        let device = device_on(&h, 161).await;
        let (admin, csrf) = login(&h, "anna").await;
        let path = format!("/devices/{device}/sensors/interfaces");
        for body in [
            format!("csrf_token={csrf}"),
            format!("csrf_token={csrf}&t_abc=on&x_1=on&s_-5=on"),
        ] {
            let reply = send(&h.app, post(&path, Some(&admin), &body)).await;
            assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        }
        assert!(
            store::list_sensors(&h.state.pool, device)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn discovery_failures_are_shown_not_raised() {
        let silent = FakeAgent::spawn("other", "x", 1).await; // wrong community: no answers
        let h = harness(true).await;
        let device = device_on(&h, silent.port).await;
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(
            &h.app,
            post(
                &format!("/devices/{device}/discover"),
                Some(&admin),
                &csrf_only(&csrf),
            ),
        )
        .await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains("Keine Antwort"), "{}", page.body);
        assert!(!page.body.contains("public"), "no secrets on the page");
    }

    #[tokio::test]
    async fn toggle_and_delete_work_and_paused_sensors_are_shown_as_paused() {
        let h = harness(true).await;
        let device = device_on(&h, 161).await;
        let (admin, csrf) = login(&h, "anna").await;
        let id = store::create_sensor(&h.state.pool, device, "Ping", &SensorSpec::Ping)
            .await
            .unwrap();

        let reply = send(
            &h.app,
            post(
                &format!("/sensors/{id}/toggle"),
                Some(&admin),
                &csrf_only(&csrf),
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/devices/{device}/sensors?msg=sensor_paused")
        );
        let page = send(
            &h.app,
            get(&format!("/devices/{device}/sensors"), Some(&admin)),
        )
        .await;
        assert!(page.body.contains("Inaktiv") && page.body.contains("Inaktive Sensoren (1)"));
        let reply = send(
            &h.app,
            post(
                &format!("/sensors/{id}/toggle"),
                Some(&admin),
                &csrf_only(&csrf),
            ),
        )
        .await;
        assert!(reply.location().ends_with("msg=sensor_resumed"));

        let reply = send(
            &h.app,
            post(
                &format!("/sensors/{id}/delete"),
                Some(&admin),
                &csrf_only(&csrf),
            ),
        )
        .await;
        assert!(reply.location().ends_with("msg=sensor_deleted"));
        assert!(
            store::get_sensor(&h.state.pool, id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn the_overview_shows_device_status_and_sensor_counts() {
        let h = harness(true).await;
        let device = device_on(&h, 161).await;
        let (admin, _) = login(&h, "anna").await;
        let home = send(&h.app, get("/", Some(&admin))).await;
        assert!(home.body.contains("keine Sensoren") && home.body.contains("0 Sensoren"));

        let sensor = store::create_sensor(&h.state.pool, device, "Ping", &SensorSpec::Ping)
            .await
            .unwrap();
        crate::storage::status::record_poll(
            &h.state.pool,
            sensor,
            1_000,
            &[],
            &crate::status::Assessment::Failed("weg".into()),
        )
        .await
        .unwrap();
        crate::storage::status::record_poll(
            &h.state.pool,
            sensor,
            2_000,
            &[],
            &crate::status::Assessment::Failed("weg".into()),
        )
        .await
        .unwrap();
        let home = send(&h.app, get("/", Some(&admin))).await;
        assert!(home.body.contains("status-down") && home.body.contains("Ausfall"));
        assert!(home.body.contains("1 Sensor<") && home.body.contains("1 Ausfall"));
        let page = send(
            &h.app,
            get(&format!("/devices/{device}/sensors"), Some(&admin)),
        )
        .await;
        // The failure is long past: no current value, and nothing within the last hour.
        assert!(page.body.contains("weg"));
        assert!(!page.body.contains("class=\"sparkline\""));
    }

    #[tokio::test]
    async fn active_sensors_show_their_value_and_the_last_hour() {
        use crate::storage::sensors::{Sample, insert_samples};
        let h = harness(true).await;
        let device = device_on(&h, 161).await;
        let (viewer, _) = login(&h, "bert").await;
        let now = now_ms();
        let ping = store::create_sensor(&h.state.pool, device, "Ping", &SensorSpec::Ping)
            .await
            .unwrap();
        let rtt = store::get_sensor(&h.state.pool, ping)
            .await
            .unwrap()
            .unwrap()
            .channels[0]
            .id;
        let samples: Vec<Sample> = [(2 * 3_600_000, 99.0), (50 * 60_000, 20.0), (30_000, 54.41)]
            .into_iter()
            .map(|(age, value)| Sample {
                channel_id: rtt,
                ts_ms: now - age,
                value,
            })
            .collect();
        insert_samples(&h.state.pool, &samples).await.unwrap();
        let uptime = store::create_sensor(&h.state.pool, device, "Uptime", &SensorSpec::Uptime)
            .await
            .unwrap();
        for (age, assessment) in [
            (
                2 * 3_600_000,
                crate::status::Assessment::Healthy("OK".into()),
            ),
            (30 * 60_000, crate::status::Assessment::Failed("weg".into())),
        ] {
            crate::storage::status::record_poll(&h.state.pool, uptime, now - age, &[], &assessment)
                .await
                .unwrap();
        }
        let inactive = store::create_sensor(&h.state.pool, device, "Leer", &SensorSpec::Uptime)
            .await
            .unwrap();
        store::set_enabled(&h.state.pool, inactive, false)
            .await
            .unwrap();

        let page = send(
            &h.app,
            get(&format!("/devices/{device}/sensors"), Some(&viewer)),
        )
        .await;
        assert!(page.body.contains("<th class=\"col-trend\">1 Stunde</th>"));
        // Ping: the latest value, its channel as tooltip, a line linking to the detail page.
        assert!(
            page.body
                .contains("title=\"Antwortzeit: 54,41 ms\"><span>54,41 ms</span></td>"),
            "{}",
            page.body
        );
        assert!(
            page.body
                .contains(&format!("<a class=\"trend\" href=\"/sensors/{ping}\""))
        );
        assert_eq!(
            page.body.matches("<path class=\"spark-1\" d=\"M").count(),
            1
        );
        // Uptime: up, then a warning half an hour ago.
        assert!(page.body.contains("class=\"statusbar spark-band\""));
        assert!(
            page.body.contains("<rect class=\"seg-up\"")
                && page.body.contains("<rect class=\"seg-warning\"")
        );
        // Inactive sensors get no chart.
        assert_eq!(page.body.matches("class=\"trend\"").count(), 2);

        let english = crate::storage::users::set_language(&h.state.pool, 2, Some(Lang::En));
        english.await.unwrap();
        let page = send(
            &h.app,
            get(&format!("/devices/{device}/sensors"), Some(&viewer)),
        )
        .await;
        assert!(
            page.body
                .contains("title=\"Response time: 54.41 ms\"><span>54.41 ms</span></td>")
        );
        assert!(page.body.contains("aria-label=\"Last hour: Ping\""));
    }

    #[tokio::test]
    async fn bulk_actions_change_only_the_chosen_sensors_of_this_device() {
        let h = harness(true).await;
        let device = device_on(&h, 161).await;
        let other = save_device(
            &h.state.pool,
            None,
            DeviceInput {
                name: "Anderes".to_owned(),
                host: "127.0.0.2".to_owned(),
                group_id: None,
                profile_id: Some(add_v2c_profile(&h, "Zweit", "public").await),
                polling: Polling {
                    port: 161,
                    interval_secs: 60,
                    timeout_secs: 2,
                    retries: 0,
                },
            },
        )
        .await
        .unwrap();
        let mut ids = Vec::new();
        for (dev, name) in [(device, "A"), (device, "B"), (device, "C"), (other, "X")] {
            ids.push(
                store::create_sensor_with_state(
                    &h.state.pool,
                    dev,
                    name,
                    &SensorSpec::Uptime,
                    false,
                )
                .await
                .unwrap(),
            );
        }
        let (admin, csrf) = login(&h, "anna").await;
        let path = format!("/devices/{device}/sensors/bulk");
        let enabled = |names: &'static [&'static str]| {
            let pool = h.state.pool.clone();
            async move {
                let mut on: Vec<String> = Vec::new();
                for dev in [device, other] {
                    for s in store::list_sensors(&pool, dev).await.unwrap() {
                        if s.enabled {
                            on.push(s.name);
                        }
                    }
                }
                on.sort();
                assert_eq!(on, names);
            }
        };

        let page = send(
            &h.app,
            get(&format!("/devices/{device}/sensors"), Some(&admin)),
        )
        .await;
        assert!(page.body.contains("Inaktive Sensoren (3)"));

        // The sensor of the other device is ignored even when named.
        let reply = send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!(
                    "csrf_token={csrf}&action=activate&s_{}=on&s_{}=on",
                    ids[0], ids[3]
                ),
            ),
        )
        .await;
        assert!(reply.location().ends_with("msg=sensors_activated"));
        enabled(&["A"]).await;

        send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!("csrf_token={csrf}&action=activate_all"),
            ),
        )
        .await;
        enabled(&["A", "B", "C"]).await;

        send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!("csrf_token={csrf}&action=deactivate&s_{}=on", ids[1]),
            ),
        )
        .await;
        enabled(&["A", "C"]).await;

        let reply = send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!(
                    "csrf_token={csrf}&action=delete&s_{}=on&s_{}=on",
                    ids[1], ids[3]
                ),
            ),
        )
        .await;
        assert!(reply.location().ends_with("msg=sensors_deleted"));
        assert_eq!(
            store::list_sensors(&h.state.pool, device)
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            store::list_sensors(&h.state.pool, other)
                .await
                .unwrap()
                .len(),
            1
        );

        for body in [
            format!("csrf_token={csrf}&action=activate"),
            format!("csrf_token={csrf}&action=explode&s_{}=on", ids[0]),
        ] {
            assert_eq!(
                send(&h.app, post(&path, Some(&admin), &body)).await.status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{body}"
            );
        }
        let (viewer, vcsrf) = login(&h, "bert").await;
        assert_eq!(
            send(
                &h.app,
                post(
                    &path,
                    Some(&viewer),
                    &format!("csrf_token={vcsrf}&action=activate_all")
                )
            )
            .await
            .status,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn new_sensors_with_named_failure_values_get_suggested_ratings() {
        let h = harness(true).await;
        let device = device_on(&h, 161).await;
        let (admin, csrf) = login(&h, "anna").await;
        let path = format!("/devices/{device}/sensors");
        for (name, oid) in [
            ("Port 1", "IF-MIB%3A%3AifOperStatus.1"),
            ("Laufzeit", "sysUpTime"),
        ] {
            let reply = send(
                &h.app,
                post(
                    &path,
                    Some(&admin),
                    &format!(
                        "csrf_token={csrf}&kind=oid&name={name}&oid={oid}&mode=auto&scale=1&unit="
                    ),
                ),
            )
            .await;
            assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.body);
        }
        let sensors = store::list_sensors(&h.state.pool, device).await.unwrap();
        let port = sensors.iter().find(|s| s.name == "Port 1").unwrap();
        let states = &port.channels[0].states;
        let rating = |label: &str| states.iter().find(|s| s.label == label).map(|s| s.status);
        assert_eq!(rating("up"), Some(crate::status::Status::Up));
        assert_eq!(rating("down"), Some(crate::status::Status::Down));
        assert_eq!(rating("testing"), Some(crate::status::Status::Warning));
        let uptime = sensors.iter().find(|s| s.name == "Laufzeit").unwrap();
        assert!(
            uptime.channels[0].states.is_empty(),
            "plain numbers stay unrated"
        );
    }

    #[tokio::test]
    async fn sensor_pages_and_their_errors_are_shown_in_english() {
        let h = harness(true).await;
        let device = save_device(
            &h.state.pool,
            None,
            DeviceInput {
                name: "Printer".to_owned(),
                host: "192.0.2.9".to_owned(),
                group_id: None,
                profile_id: None,
                polling: Polling::default(),
            },
        )
        .await
        .unwrap();
        crate::storage::users::set_language(&h.state.pool, 1, Some(Lang::En))
            .await
            .unwrap();
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(
            &h.app,
            get(&format!("/devices/{device}/sensors"), Some(&admin)),
        )
        .await;
        for text in [
            "No sensors yet.",
            "Add sensor",
            "no SNMP, ping only",
            "every 60 s",
        ] {
            assert!(page.body.contains(text), "{text} missing");
        }
        let refused = send(
            &h.app,
            post(
                &format!("/devices/{device}/sensors"),
                Some(&admin),
                &format!("csrf_token={csrf}&kind=bogus"),
            ),
        )
        .await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(refused.body.contains("Please choose a sensor type."));
        let values = send(
            &h.app,
            post(
                &format!("/devices/{device}/values"),
                Some(&admin),
                &format!("csrf_token={csrf}&object=noSuchTable"),
            ),
        )
        .await;
        assert!(
            values
                .body
                .contains("“noSuchTable” is not known in any loaded MIB."),
            "{}",
            values.body
        );
        let order = send(&h.app, get("/groups/0/order", Some(&admin))).await;
        assert!(
            order.body.contains("Order of the top level")
                && order.body.contains("Sort automatically")
        );
    }
}
