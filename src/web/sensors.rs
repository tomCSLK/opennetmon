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
use crate::sensors::snmp::OidMode;
use crate::sensors::spec::{SensorSpec, parse_oid};
use crate::snmp::{SnmpClient, SnmpTarget};
use crate::status::{Status, device_status};
use crate::storage::sensors::{self as store, SensorRecord};
use crate::storage::{credentials, devices, status as status_store};
use crate::web::auth::{CsrfForm, CurrentUser};
use crate::web::overview::FlashQuery;
use crate::web::{AppState, Layout, flash_message, time_html};

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
    status_label: &'static str,
    status_class: &'static str,
    message: String,
    last_poll: String,
    enabled: bool,
}

#[derive(Clone)]
struct SensorFormValues {
    kind: String,
    name: String,
    oid: String,
    mode: String,
    scale: String,
}

impl Default for SensorFormValues {
    fn default() -> Self {
        Self {
            kind: "ping".to_owned(),
            name: String::new(),
            oid: String::new(),
            mode: "gauge".to_owned(),
            scale: "1".to_owned(),
        }
    }
}

#[derive(Template)]
#[template(path = "sensors.html")]
struct SensorsTemplate {
    layout: Layout,
    device: Device,
    device_status_label: &'static str,
    device_status_class: &'static str,
    rows: Vec<SensorRowView>,
    flash: Option<&'static str>,
    error: Option<String>,
    form: SensorFormValues,
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
    layout: Layout,
    device: Device,
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
}

impl SensorForm {
    fn values(&self) -> SensorFormValues {
        SensorFormValues {
            kind: self.kind.clone(),
            name: self.name.clone(),
            oid: self.oid.clone(),
            mode: self.mode.clone(),
            scale: self.scale.clone(),
        }
    }

    /// Name and configuration for the chosen sensor type.
    fn to_spec(&self) -> Result<(String, SensorSpec)> {
        let invalid = |m: &str| Error::Invalid(m.to_owned());
        let (default_name, spec) = match self.kind.as_str() {
            "ping" => ("Ping", SensorSpec::Ping),
            "uptime" => ("Uptime", SensorSpec::Uptime),
            "oid" => {
                let oid = parse_oid(&self.oid).map_err(invalid)?;
                let mode = match self.mode.as_str() {
                    "gauge" => OidMode::Gauge,
                    "counter" => OidMode::Counter,
                    _ => return Err(invalid("Bitte den Modus (Wert oder Zähler) wählen.")),
                };
                let scale = match self.scale.trim() {
                    "" => 1.0,
                    text => text
                        .replace(',', ".")
                        .parse::<f64>()
                        .map_err(|_| invalid("Der Faktor muss eine Zahl sein."))?,
                };
                ("", SensorSpec::Oid { oid, mode, scale })
            }
            _ => return Err(invalid("Bitte einen Sensortyp wählen.")),
        };
        let name = match self.name.trim() {
            "" => default_name,
            name => name,
        };
        if name.is_empty() {
            return Err(invalid("Bitte einen Namen angeben."));
        }
        Ok((name.to_owned(), spec))
    }
}

async fn load_device(state: &AppState, id: i64) -> Result<Device> {
    devices::get_device(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)
}

async fn render_sensors(
    state: &AppState,
    user: &CurrentUser,
    device: Device,
    flash: Option<&'static str>,
    error: Option<String>,
    form: SensorFormValues,
) -> Result<String> {
    let records = store::list_sensors(&state.pool, device.id).await?;
    let states = status_store::list_states(&state.pool).await?;
    let overall = device_status(
        records
            .iter()
            .map(|r| (r.enabled, states.get(&r.id).map(|s| s.status))),
    );
    let rows = records
        .iter()
        .map(|r| {
            let state = states.get(&r.id);
            let status = if r.enabled {
                state.map_or(Status::Unknown, |s| s.status)
            } else {
                Status::Paused
            };
            SensorRowView {
                id: r.id,
                name: r.name.clone(),
                description: r.spec.describe(),
                status_label: status.label(),
                status_class: status.as_str(),
                message: state.map(|s| s.message.clone()).unwrap_or_default(),
                last_poll: state.map_or_else(String::new, |s| time_html(s.last_poll_ms)),
                enabled: r.enabled,
            }
        })
        .collect();
    let page = SensorsTemplate {
        layout: user.layout(),
        device,
        device_status_label: overall.label(),
        device_status_class: overall.as_str(),
        rows,
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
            flash_message(query.msg.as_deref()),
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
    let outcome = match form.to_spec() {
        Ok((name, spec)) => store::create_sensor(&state.pool, id, &name, &spec).await,
        Err(e) => Err(e),
    };
    match outcome {
        Ok(sensor) => {
            tracing::info!(user = %user.username, device = id, sensor, "sensor created");
            Ok(Redirect::to(&format!("/devices/{id}/sensors?msg=sensor_created")).into_response())
        }
        Err(Error::Invalid(message) | Error::Conflict(message)) => {
            let html =
                render_sensors(&state, &user, device, None, Some(message), form.values()).await?;
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
    let creds = credentials::load_credentials(&state.pool, &state.secrets, device.profile_id)
        .await?
        .ok_or_else(|| Error::Internal("device refers to a missing profile".to_owned()))?;
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
        Err(e) => (Vec::new(), Some(e.user_message())),
    };
    let page = DiscoverTemplate {
        layout: user.layout(),
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
        return Err(Error::Invalid("Das Formular ist zu groß.".to_owned()));
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
        return Err(Error::Invalid(
            "Bitte mindestens ein Interface auswählen.".to_owned(),
        ));
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
                profile_id: profile,
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
        assert_eq!(names, ["Laufzeit", "Ping", "Temperatur"]);
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
                "kind=oid&name=X&oid=sysUpTime.0&mode=gauge&scale=1",
                "OID ist ungültig",
            ),
            ("kind=oid&name=&oid=1.3.6.1&mode=gauge&scale=1", "Namen"),
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
        assert!(page.body.contains("Pausiert") && page.body.contains("Fortsetzen"));
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
        assert!(home.body.contains("keine Sensoren") && home.body.contains("Sensoren (0)"));

        let sensor = store::create_sensor(&h.state.pool, device, "Ping", &SensorSpec::Ping)
            .await
            .unwrap();
        crate::storage::status::record_poll(
            &h.state.pool,
            sensor,
            1_000,
            &[],
            &crate::status::Assessment::Failed("weg".to_owned()),
        )
        .await
        .unwrap();
        crate::storage::status::record_poll(
            &h.state.pool,
            sensor,
            2_000,
            &[],
            &crate::status::Assessment::Failed("weg".to_owned()),
        )
        .await
        .unwrap();
        let home = send(&h.app, get("/", Some(&admin))).await;
        assert!(home.body.contains("status-down") && home.body.contains("Ausfall"));
        assert!(home.body.contains("Sensoren (1)"));
        let page = send(
            &h.app,
            get(&format!("/devices/{device}/sensors"), Some(&admin)),
        )
        .await;
        assert!(page.body.contains("weg") && page.body.contains("1970-01-01 00:00:02 UTC"));
    }
}
