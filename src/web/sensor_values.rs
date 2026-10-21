//! "Find values" for a device (admin only): walks known tables (PoE, CPU, memory, hardware
//! sensors, …) or a table/group the admin names, and creates the ticked values as custom OID
//! sensors. New sensors start **inactive** unless the admin asks otherwise.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use askama::Template;
use axum::Form;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::devices::Device;
use crate::error::{Error, Result};
use crate::i18n::{Lang, Msg};
use crate::mib::{self, MibIndex, ObjectKind};
use crate::sensors::snmp::OidMode;
use crate::sensors::spec::{SensorSpec, parse_oid};
use crate::snmp::{SnmpClient, SnmpTarget};
use crate::storage::devices;
use crate::storage::sensors as store;
use crate::value_discovery::{self, Group, Target};
use crate::web::auth::CurrentUser;
use crate::web::{AppState, Crumb, Layout, group_trail};

/// Discovery never blocks a request for long, whatever the device is configured to.
const MAX_TIMEOUT_SECS: u32 = 10;
const MAX_RETRIES: u32 = 1;
/// Upper bound of fields in one "create" form.
const MAX_FIELDS: usize = 20_000;

struct CandidateView {
    key: usize,
    oid: String,
    name: String,
    value: String,
    scale: String,
    unit: String,
    mode: &'static str,
    selected: bool,
    exists: bool,
}

struct GroupView {
    title: String,
    source: String,
    note: Option<String>,
    candidates: Vec<CandidateView>,
}

#[derive(Template)]
#[template(path = "sensor_values.html")]
struct PageTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    device: Device,
    /// Groups above the device, for the breadcrumbs.
    trail: Vec<Crumb>,
    groups: Vec<GroupView>,
    object: String,
    failure: Option<String>,
    searched_catalog: bool,
}

#[derive(Deserialize)]
pub struct DiscoverForm {
    csrf_token: String,
    /// A table or group to search instead of the catalog, e.g. `tpPoePortConfigTable`.
    #[serde(default)]
    object: String,
}

async fn load_device(state: &AppState, id: i64) -> Result<Device> {
    devices::get_device(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)
}

/// The table or group named by the admin (`name` or `MODULE::name`).
fn find_object<'a>(
    index: &'a MibIndex,
    input: &str,
) -> std::result::Result<&'a mib::MibObject, Msg> {
    let input = input.trim();
    let (module, name) = match input.split_once("::") {
        Some((m, n)) => (Some(m.trim()), n.trim()),
        None => (None, input),
    };
    let found = index.by_name(name, module);
    let object = found
        .first()
        .copied()
        .ok_or_else(|| Msg::new("mib-name-unknown").with("name", name))?;
    match object.kind {
        ObjectKind::Table | ObjectKind::Node => Ok(object),
        ObjectKind::Row => index
            .get_oid(&object.oid[..object.oid.len() - 1])
            .ok_or_else(|| Msg::new("values-row-without-table")),
        _ => Err(Msg::new("values-not-a-table")
            .with("name", object.name.as_str())
            .with("kind", Msg::new(object.kind.label_id()))),
    }
}

fn existing_oids(sensors: &[store::SensorRecord]) -> HashSet<Vec<u64>> {
    sensors
        .iter()
        .filter_map(|s| match &s.spec {
            SensorSpec::Oid { oid, .. } => Some(oid.clone()),
            _ => None,
        })
        .collect()
}

fn views(groups: Vec<Group>, existing: &HashSet<Vec<u64>>) -> Vec<GroupView> {
    let mut key = 0;
    groups
        .into_iter()
        .map(|g| GroupView {
            title: g.title,
            source: g.source,
            note: g.note,
            candidates: g
                .candidates
                .into_iter()
                .map(|c| {
                    key += 1;
                    let exists = existing.contains(&c.oid);
                    CandidateView {
                        key,
                        oid: mib::dotted(&c.oid),
                        name: c.name,
                        value: c.value,
                        scale: format!("{}", c.scale),
                        unit: c.unit.unwrap_or_default(),
                        mode: match c.mode {
                            OidMode::Gauge => "gauge",
                            OidMode::Counter => "counter",
                        },
                        selected: c.selected && !exists,
                        exists,
                    }
                })
                .collect(),
        })
        .collect()
}

pub async fn discover(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<DiscoverForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let device = load_device(&state, id).await?;
    let index = state.mib_index();
    let object = form.object.trim().to_owned();
    let trail = group_trail(&state.pool, device.group_id).await?;
    let page = |groups, failure, searched_catalog| PageTemplate {
        layout: user.layout(),
        t: user.lang,
        trail: trail.clone(),
        device: device.clone(),
        groups,
        object: object.clone(),
        failure,
        searched_catalog,
    };

    let targets = if object.is_empty() {
        value_discovery::catalog_targets()
    } else {
        match find_object(&index, &object) {
            Ok(o) => vec![Target::Object(o)],
            Err(message) => {
                let html = page(Vec::new(), Some(user.lang.msg(&message)), false).render()?;
                return Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response());
            }
        }
    };
    let creds = crate::web::snmp_credentials(&state, &device).await?;
    let target = SnmpTarget {
        host: device.host.clone(),
        port: device.polling.port,
        timeout: Duration::from_secs(u64::from(device.polling.timeout_secs.min(MAX_TIMEOUT_SECS))),
        retries: device.polling.retries.min(MAX_RETRIES),
    };
    let outcome = async {
        let mut client = SnmpClient::connect(&target, &creds).await?;
        value_discovery::discover(user.lang, &mut client, &index, &targets).await
    }
    .await;
    tracing::info!(user = %user.username, device = id, ok = outcome.is_ok(), "value discovery");
    let existing = existing_oids(&store::list_sensors(&state.pool, id).await?);
    let html = match outcome {
        Ok(groups) => page(views(groups, &existing), None, object.is_empty()).render()?,
        Err(e) => page(
            Vec::new(),
            Some(user.lang.msg(&e.user_message())),
            object.is_empty(),
        )
        .render()?,
    };
    Ok(Html(html).into_response())
}

/// One ticked proposal from the form.
struct Proposal {
    name: String,
    spec: SensorSpec,
}

fn proposals(fields: &HashMap<String, String>) -> Result<Vec<Proposal>> {
    let get = |prefix: &str, key: &str| {
        fields
            .get(&format!("{prefix}_{key}"))
            .map(|s| s.trim().to_owned())
            .unwrap_or_default()
    };
    let mut keys: Vec<usize> = fields
        .iter()
        .filter(|(k, v)| k.starts_with("c_") && v.as_str() == "on")
        .filter_map(|(k, _)| k[2..].parse().ok())
        .collect();
    keys.sort_unstable();
    keys.into_iter()
        .map(|k| {
            let k = k.to_string();
            let oid = parse_oid(&get("o", &k)).map_err(Error::invalid)?;
            let name = get("n", &k);
            if name.is_empty() {
                return Err(Error::invalid(Msg::new("values-name-missing")));
            }
            let scale = get("f", &k).replace(',', ".").parse::<f64>().map_err(|_| {
                Error::invalid(
                    Msg::new("channel-error")
                        .with("channel", name.as_str())
                        .with("message", Msg::new("sensors-scale-not-number")),
                )
            })?;
            let unit = Some(get("u", &k)).filter(|u| !u.is_empty());
            let mode = match get("m", &k).as_str() {
                "counter" => OidMode::Counter,
                _ => OidMode::Gauge,
            };
            let spec = SensorSpec::Oid {
                oid,
                mode,
                scale,
                unit,
            };
            spec.validate().map_err(|m| {
                Error::invalid(
                    Msg::new("channel-error")
                        .with("channel", name.as_str())
                        .with("message", m),
                )
            })?;
            Ok(Proposal { name, spec })
        })
        .collect()
}

pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(fields): Form<HashMap<String, String>>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(fields.get("csrf_token").map_or("", String::as_str))?;
    load_device(&state, id).await?;
    if fields.len() > MAX_FIELDS {
        return Err(Error::invalid(Msg::new("form-too-large")));
    }
    let wanted = proposals(&fields)?;
    if wanted.is_empty() {
        return Err(Error::invalid(Msg::new("values-select-one")));
    }
    let activate = fields.get("activate").is_some_and(|v| v == "on");
    let mut existing = existing_oids(&store::list_sensors(&state.pool, id).await?);
    let mut created = 0usize;
    for p in wanted {
        let SensorSpec::Oid { oid, .. } = &p.spec else {
            continue;
        };
        if !existing.insert(oid.clone()) {
            continue; // already monitored
        }
        let mut result =
            store::create_sensor_with_state(&state.pool, id, &p.name, &p.spec, activate).await;
        // Same name already used on this device: number it.
        let mut n = 2;
        while matches!(result, Err(Error::Conflict(_))) && n < 100 {
            let suffix = format!(" ({n})");
            let base: String = p
                .name
                .chars()
                .take(crate::devices::MAX_NAME_LEN - suffix.chars().count())
                .collect();
            result = store::create_sensor_with_state(
                &state.pool,
                id,
                &format!("{base}{suffix}"),
                &p.spec,
                activate,
            )
            .await;
            n += 1;
        }
        crate::web::sensor_text::rate_new_sensor(&state.pool, result?, &p.spec, &state.mib_index())
            .await?;
        created += 1;
    }
    tracing::info!(user = %user.username, device = id, created, activate, "sensors created from value discovery");
    let msg = if activate {
        "values_added_active"
    } else {
        "values_added_inactive"
    };
    Ok(Redirect::to(&format!("/devices/{id}/sensors?msg={msg}")).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{DeviceInput, Polling};
    use crate::snmp::testagent::{FakeAgent, FakeValue};
    use crate::storage::devices::save_device;
    use crate::web::testutil::*;

    const HR_PROCESSOR_LOAD: &[u64] = &[1, 3, 6, 1, 2, 1, 25, 3, 3, 1, 2];

    async fn device_on(h: &Harness, port: u16) -> i64 {
        let profile = add_v2c_profile(h, "Fake", "public").await;
        save_device(
            &h.state.pool,
            None,
            DeviceInput {
                kind: Default::default(),
                name: "Server".to_owned(),
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

    async fn agent_with_cpus() -> FakeAgent {
        let agent = FakeAgent::spawn("public", "x", 1).await;
        agent.set(
            &[HR_PROCESSOR_LOAD, &[196608]].concat(),
            FakeValue::Integer(17),
        );
        agent.set(
            &[HR_PROCESSOR_LOAD, &[196609]].concat(),
            FakeValue::Integer(4),
        );
        agent
    }

    fn create_form(csrf: &str, oid: &str, name: &str, activate: bool) -> String {
        let mut form = format!(
            "csrf_token={csrf}&c_1=on&o_1={oid}&n_1={name}&f_1=1&u_1=%25&m_1=gauge&c_2=&o_2=1.3.6.1.9&n_2=x&f_2=1&u_2=&m_2=gauge"
        );
        if activate {
            form.push_str("&activate=on");
        }
        form
    }

    #[tokio::test]
    async fn known_tables_are_found_and_ticked_values_become_inactive_sensors() {
        let agent = agent_with_cpus().await;
        let h = harness(true).await;
        let device = device_on(&h, agent.port).await;
        let (admin, csrf) = login(&h, "anna").await;

        let page = send(
            &h.app,
            post(
                &format!("/devices/{device}/values"),
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(
            page.body.contains("CPU-Last (HOST-RESOURCES-MIB)"),
            "{}",
            page.body
        );
        assert!(page.body.contains("value=\"CPU-Last CPU 1\""));
        assert!(page.body.contains("17 %"));
        assert!(page.body.contains("1.3.6.1.2.1.25.3.3.1.2.196608"));

        let oid = "1.3.6.1.2.1.25.3.3.1.2.196608";
        let reply = send(
            &h.app,
            post(
                &format!("/devices/{device}/values/create"),
                Some(&admin),
                &create_form(&csrf, oid, "CPU 1", false),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        assert!(reply.location().ends_with("msg=values_added_inactive"));
        let sensors = store::list_sensors(&h.state.pool, device).await.unwrap();
        assert_eq!(sensors.len(), 1, "only ticked values are created");
        assert_eq!(sensors[0].name, "CPU 1");
        assert!(!sensors[0].enabled, "new sensors start inactive");
        assert!(matches!(
            &sensors[0].spec,
            SensorSpec::Oid { unit: Some(u), mode: OidMode::Gauge, .. } if u == "%"
        ));

        // The same OID again is skipped; the next search marks it as present.
        send(
            &h.app,
            post(
                &format!("/devices/{device}/values/create"),
                Some(&admin),
                &create_form(&csrf, oid, "CPU 1", true),
            ),
        )
        .await;
        assert_eq!(
            store::list_sensors(&h.state.pool, device)
                .await
                .unwrap()
                .len(),
            1
        );
        let page = send(
            &h.app,
            post(
                &format!("/devices/{device}/values"),
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert!(page.body.contains("vorhanden"));

        // Asked for: active at once, and a taken name is numbered.
        let reply = send(
            &h.app,
            post(
                &format!("/devices/{device}/values/create"),
                Some(&admin),
                &create_form(&csrf, "1.3.6.1.2.1.25.3.3.1.2.196609", "CPU 1", true),
            ),
        )
        .await;
        assert!(reply.location().ends_with("msg=values_added_active"));
        let sensors = store::list_sensors(&h.state.pool, device).await.unwrap();
        let second = sensors.iter().find(|s| s.name == "CPU 1 (2)").unwrap();
        assert!(second.enabled);
    }

    #[tokio::test]
    async fn a_named_table_is_searched_and_bad_names_are_explained() {
        let agent = agent_with_cpus().await;
        let h = harness(true).await;
        let device = device_on(&h, agent.port).await;
        let (admin, csrf) = login(&h, "anna").await;
        let search = |object: &str| {
            post(
                &format!("/devices/{device}/values"),
                Some(&admin),
                &format!("csrf_token={csrf}&object={object}"),
            )
        };

        let page = send(&h.app, search("HOST-RESOURCES-MIB%3A%3AhrProcessorTable")).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains("hrProcessorLoad"), "{}", page.body);
        assert!(page.body.contains("1.3.6.1.2.1.25.3.3.1.2.196609"));

        for (object, text) in [
            ("noSuchThing", "in keiner geladenen MIB"),
            ("sysDescr", "bitte eine Tabelle oder Gruppe"),
        ] {
            let page = send(&h.app, search(object)).await;
            assert_eq!(page.status, StatusCode::UNPROCESSABLE_ENTITY, "{object}");
            assert!(page.body.contains(text), "{object}");
        }
    }

    #[tokio::test]
    async fn bad_create_forms_and_viewers_are_rejected() {
        let h = harness(true).await;
        let device = device_on(&h, 161).await;
        let (admin, csrf) = login(&h, "anna").await;
        let path = format!("/devices/{device}/values/create");
        for body in [
            format!("csrf_token={csrf}"),
            create_form(&csrf, "not-an-oid", "x", false),
            create_form(&csrf, "1.3.6.1.2.1.1.3.0", "", false),
            create_form(&csrf, "1.3.6.1.2.1.1.3.0", "x", false).replace("f_1=1", "f_1=abc"),
        ] {
            let reply = send(&h.app, post(&path, Some(&admin), &body)).await;
            assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        }
        let (viewer, vcsrf) = login(&h, "bert").await;
        for target in [format!("/devices/{device}/values"), path] {
            let body = create_form(&vcsrf, "1.3.6.1.2.1.1.3.0", "x", true);
            assert_eq!(
                send(&h.app, post(&target, Some(&viewer), &body))
                    .await
                    .status,
                StatusCode::FORBIDDEN
            );
        }
        assert!(
            store::list_sensors(&h.state.pool, device)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
