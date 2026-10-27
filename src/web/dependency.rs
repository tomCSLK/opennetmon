//! The dependency setting of a group, device or sensor (admin only): `/dependency/<kind>/<id>` with
//! "inherit", "depend on a sensor" (chosen from all sensors, by device) or "no dependency", the delay
//! after recovery and, for a sensor, whether it is the master sensor of its device. See
//! [`crate::pause`] for how dependencies pause objects.

use askama::Template;
use axum::Form;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use std::collections::HashMap;

use crate::devices::{Group, flatten_groups};
use crate::error::{Error, Result};
use crate::i18n::{Lang, Msg};
use crate::pause::{Dependency, Effective, Origin, Pauses, Setting, Target, parse_delay};
use crate::storage::pauses::{self, DependencyError};
use crate::storage::{devices, groups, sensors};
use crate::web::auth::CurrentUser;
use crate::web::pause::{Object, object};
use crate::web::sensors::now_ms;
use crate::web::{AppState, Crumb, Layout};

/// "„Ping“ (Router), von der Gruppe „Netzwerk“" – what an effective dependency means, for hints.
pub fn describe(t: Lang, pauses: &Pauses, effective: &Effective) -> String {
    let Some(info) = effective.sensor.and_then(|id| pauses.sensor_info(id)) else {
        return t.get("dependency-current-none");
    };
    let on = t.get2(
        "dependency-on",
        "sensor",
        info.name.as_str(),
        "device",
        info.device.as_str(),
    );
    match origin(t, &effective.from) {
        Some(from) => format!("{on}, {from}"),
        None => on,
    }
}

fn origin(t: Lang, origin: &Origin) -> Option<String> {
    match origin {
        Origin::Device => Some(t.get("dependency-from-device")),
        Origin::Group(name) => Some(t.get1("dependency-from-group", "name", name.as_str())),
        Origin::Own | Origin::Default => None,
    }
}

fn seconds(t: Lang, secs: u32, from: &Origin) -> String {
    let value = format!("{secs} s");
    match origin(t, from) {
        Some(from) => format!("{value}, {from}"),
        None => value,
    }
}

struct SensorOption {
    id: i64,
    name: String,
    selected: bool,
}

struct DeviceOptions {
    /// The device with the path of its group ("Netzwerk › Switche › Switch Büro").
    name: String,
    sensors: Vec<SensorOption>,
}

#[derive(Template)]
#[template(path = "dependency_form.html")]
struct DependencyTemplate {
    t: Lang,
    layout: Layout,
    heading: String,
    hint: String,
    trail: Vec<Crumb>,
    action: String,
    cancel_href: String,
    /// `parent`, `sensor` or `none`.
    choice: String,
    /// What "inherit" means here.
    inherited: String,
    devices: Vec<DeviceOptions>,
    delay: String,
    delay_inherited: String,
    /// `Some(checked)` for a sensor.
    master: Option<bool>,
    error: Option<String>,
}

/// The values of the form, as stored or as entered.
struct Values {
    choice: String,
    sensor: Option<i64>,
    delay: String,
    master: bool,
}

fn href(obj: &Object, msg: Option<&str>) -> String {
    let query = msg.map(|m| format!("?msg={m}")).unwrap_or_default();
    match obj.target {
        Target::Group(id) => format!("/groups/{id}{query}"),
        Target::Device(id) => format!("/devices/{id}/sensors{query}"),
        Target::Sensor(id) => format!("/sensors/{id}{query}"),
    }
}

async fn render(
    state: &AppState,
    user: &CurrentUser,
    obj: &Object,
    pauses: &Pauses,
    values: Values,
    error: Option<String>,
) -> Result<String> {
    let t = user.lang;
    let inherited = pauses.inherited(obj.target);
    let groups = groups::list_groups(&state.pool).await?;
    let paths = group_paths(&groups);
    let mut devices = devices::list_devices(&state.pool).await?;
    in_tree_order(&groups, &mut devices, |d| d.group_id);
    let mut devices: Vec<(i64, DeviceOptions)> = devices
        .into_iter()
        .map(|d| {
            let name = match d.group_id.and_then(|g| paths.get(&g)) {
                Some(path) => format!("{path} › {}", d.name),
                None => d.name,
            };
            let options = DeviceOptions {
                name,
                sensors: Vec::new(),
            };
            (d.id, options)
        })
        .collect();
    for s in sensors::list_all_sensors(&state.pool).await? {
        // A sensor never depends on itself.
        if obj.target == Target::Sensor(s.id) {
            continue;
        }
        if let Some((_, device)) = devices.iter_mut().find(|(id, _)| *id == s.device_id) {
            device.sensors.push(SensorOption {
                id: s.id,
                selected: values.sensor == Some(s.id),
                name: s.name,
            });
        }
    }
    let devices: Vec<DeviceOptions> = devices
        .into_iter()
        .map(|(_, d)| d)
        .filter(|d| !d.sensors.is_empty())
        .collect();
    let page = DependencyTemplate {
        layout: user.layout(),
        heading: t.get1("dependency-heading", "name", obj.name.as_str()),
        hint: match obj.target {
            Target::Group(_) => t.get("dependency-hint-group"),
            Target::Device(_) => t.get("dependency-hint-device"),
            Target::Sensor(_) => String::new(),
        },
        trail: obj.trail.clone(),
        action: format!("/dependency/{}/{}", obj.target.kind(), obj.target.id()),
        cancel_href: href(obj, None),
        choice: values.choice,
        inherited: describe(t, pauses, &inherited),
        devices,
        delay: values.delay,
        delay_inherited: seconds(t, inherited.delay_secs, &inherited.delay_from),
        master: matches!(obj.target, Target::Sensor(_)).then_some(values.master),
        error,
        t,
    };
    Ok(page.render()?)
}

fn target(kind: &str, id: i64) -> Result<Target> {
    Target::parse(kind, id).ok_or(Error::NotFound)
}

pub async fn form(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((kind, id)): Path<(String, i64)>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let target = target(&kind, id)?;
    let obj = object(&state, user.lang, target).await?;
    let pauses = pauses::load_all(&state.pool, now_ms()).await?;
    let setting = pauses.setting(target);
    let values = Values {
        choice: setting.dependency.as_str().to_owned(),
        sensor: setting.dependency.sensor_id(),
        delay: setting
            .delay_secs
            .map_or_else(String::new, |d| d.to_string()),
        master: match target {
            Target::Sensor(id) => pauses.sensor_info(id).is_some_and(|i| i.master),
            _ => false,
        },
    };
    Ok(Html(
        render(&state, &user, &obj, &pauses, values, None).await?,
    ))
}

/// Deliberately no `Debug`, consistent with the other forms.
#[derive(Deserialize)]
pub struct DependencyForm {
    csrf_token: String,
    #[serde(default)]
    dependency: String,
    #[serde(default)]
    sensor: String,
    #[serde(default)]
    delay: String,
    #[serde(default)]
    master: Option<String>,
}

fn parse(form: &DependencyForm) -> std::result::Result<Setting, Msg> {
    let dependency = match form.dependency.as_str() {
        "parent" => Dependency::Parent,
        "none" => Dependency::None,
        "sensor" => Dependency::Sensor(
            form.sensor
                .trim()
                .parse()
                .map_err(|_| Msg::new("dependency-sensor-required"))?,
        ),
        _ => return Err(Msg::new("dependency-choice-invalid")),
    };
    Ok(Setting {
        dependency,
        delay_secs: parse_delay(&form.delay)?,
    })
}

pub async fn save(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((kind, id)): Path<(String, i64)>,
    Form(form): Form<DependencyForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let target = target(&kind, id)?;
    let obj = object(&state, user.lang, target).await?;
    let master = matches!(target, Target::Sensor(_)).then_some(form.master.is_some());
    let now = now_ms();
    let result = match parse(&form) {
        Ok(setting) => pauses::set_dependency(&state.pool, target, setting, master, now)
            .await
            .map(|()| setting),
        Err(msg) => Err(DependencyError::Invalid(msg)),
    };
    let msg = match result {
        Ok(setting) => {
            tracing::info!(
                user = %user.username,
                kind,
                id,
                dependency = setting.dependency.as_str(),
                sensor = ?setting.dependency.sensor_id(),
                delay = ?setting.delay_secs,
                ?master,
                "dependency saved"
            );
            return Ok(Redirect::to(&href(&obj, Some("dependency_saved"))).into_response());
        }
        Err(DependencyError::Invalid(msg)) => msg,
        Err(DependencyError::Storage(e)) => return Err(e),
    };
    let pauses = pauses::load_all(&state.pool, now).await?;
    let values = Values {
        choice: form.dependency.clone(),
        sensor: form.sensor.trim().parse().ok(),
        delay: form.delay.clone(),
        master: form.master.is_some(),
    };
    let page = render(
        &state,
        &user,
        &obj,
        &pauses,
        values,
        Some(user.lang.msg(&msg)),
    )
    .await?;
    Ok((axum::http::StatusCode::UNPROCESSABLE_ENTITY, Html(page)).into_response())
}

/// Sorts `items` like the device tree of the overview: groups depth-first, ungrouped last. The sort is
/// stable, so items of one group keep their order (`list_devices` orders by position within a group,
/// which across groups would mix them).
fn in_tree_order<T>(groups: &[Group], items: &mut [T], group_of: impl Fn(&T) -> Option<i64>) {
    let rank: HashMap<i64, usize> = flatten_groups(groups)
        .iter()
        .enumerate()
        .map(|(i, node)| (node.group.id, i))
        .collect();
    items.sort_by_key(|item| {
        group_of(item)
            .and_then(|g| rank.get(&g).copied())
            .unwrap_or(usize::MAX)
    });
}

/// "Netzwerk › Switche" for every group.
fn group_paths(groups: &[Group]) -> HashMap<i64, String> {
    let mut paths: HashMap<i64, String> = HashMap::new();
    // Depth-first, so the parent's path is always known first.
    for node in flatten_groups(groups) {
        let group = node.group;
        let path = match group.parent_id.and_then(|p| paths.get(&p)) {
            Some(parent) => format!("{parent} › {}", group.name),
            None => group.name,
        };
        paths.insert(group.id, path);
    }
    paths
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;
    use crate::devices::{DeviceInput, Polling};
    use crate::sensors::spec::SensorSpec;
    use crate::storage::{devices::save_device, groups};
    use crate::web::testutil::*;

    fn g(id: i64, name: &str, parent_id: Option<i64>) -> Group {
        Group {
            id,
            name: name.to_owned(),
            parent_id,
        }
    }

    #[test]
    fn sensors_to_choose_are_ordered_like_the_device_tree() {
        // As stored: siblings in display order, "Netzwerk" with "Switche" before "Router" by hand.
        let groups = [
            g(1, "IOT", None),
            g(2, "Netzwerk", None),
            g(3, "Switche", Some(2)),
            g(4, "Router", Some(2)),
        ];
        // Ordered by their position within each group, which mixes the groups.
        let mut devices = vec![
            ("Switch Keller", Some(3)),
            ("Router", Some(4)),
            ("Laptop", None),
            ("Tibber", Some(1)),
            ("Switch Wohnen", Some(3)),
            ("FritzBox", Some(4)),
        ];
        in_tree_order(&groups, &mut devices, |d| d.1);
        let names: Vec<&str> = devices.iter().map(|d| d.0).collect();
        assert_eq!(
            names,
            [
                "Tibber",
                "Switch Keller",
                "Switch Wohnen",
                "Router",
                "FritzBox",
                "Laptop"
            ]
        );
        let paths = group_paths(&groups);
        assert_eq!(paths[&3], "Netzwerk › Switche");
        assert_eq!(paths[&1], "IOT");
    }

    async fn device(h: &Harness, name: &str, group: Option<i64>) -> i64 {
        save_device(
            &h.state.pool,
            None,
            DeviceInput {
                kind: Default::default(),
                name: name.to_owned(),
                host: "192.0.2.1".to_owned(),
                group_id: group,
                profile_id: None,
                polling: Polling::default(),
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn admins_make_a_group_depend_on_the_router_ping() {
        let h = harness(true).await;
        let group = groups::create_group(&h.state.pool, "Switche", None)
            .await
            .unwrap();
        let router = device(&h, "Router", None).await;
        let switch = device(&h, "Switch", Some(group)).await;
        let router_ping = sensors::create_sensor(&h.state.pool, router, "Ping", &SensorSpec::Ping)
            .await
            .unwrap();
        let switch_ping = sensors::create_sensor(&h.state.pool, switch, "Ping", &SensorSpec::Ping)
            .await
            .unwrap();
        let (viewer, vcsrf) = login(&h, "bert").await;
        let (admin, csrf) = login(&h, "anna").await;
        let path = format!("/dependency/group/{group}");

        assert_eq!(
            send(&h.app, get(&path, Some(&viewer))).await.status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(
                &h.app,
                post(
                    &path,
                    Some(&viewer),
                    &format!("csrf_token={vcsrf}&dependency=none")
                )
            )
            .await
            .status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(&h.app, get("/dependency/probe/1", Some(&admin)))
                .await
                .status,
            StatusCode::NOT_FOUND
        );
        let page = send(&h.app, get(&path, Some(&admin))).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(
            page.body.contains("Abhängigkeit für „Switche“"),
            "{}",
            page.body
        );
        assert!(page.body.contains("derzeit: keine Abhängigkeit"));
        // Grouped devices with their group, in tree order, ungrouped ones last.
        let switch_at = page.body.find("<optgroup label=\"Switche › Switch\">");
        let router_at = page.body.find("<optgroup label=\"Router\">");
        assert!(switch_at.is_some() && router_at.is_some(), "{}", page.body);
        assert!(switch_at < router_at);
        assert!(
            !page.body.contains("name=\"master\""),
            "only sensors have a master"
        );

        // Errors are explained and the input is kept.
        for (form, message) in [
            ("dependency=sensor&sensor=", "Bitte einen Sensor wählen."),
            ("dependency=x", "Bitte eine der Möglichkeiten wählen."),
            (
                "dependency=parent&delay=99999",
                "eine ganze Zahl von 0 bis 3600 Sekunden",
            ),
        ] {
            let reply = send(
                &h.app,
                post(&path, Some(&admin), &format!("csrf_token={csrf}&{form}")),
            )
            .await;
            assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{form}");
            assert!(reply.body.contains(message), "{form}: {}", reply.body);
        }

        let reply = send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!("csrf_token={csrf}&dependency=sensor&sensor={router_ping}&delay=120"),
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/groups/{group}?msg=dependency_saved")
        );
        let page = send(&h.app, get(&path, Some(&admin))).await;
        assert!(
            page.body
                .contains(&format!("<option value=\"{router_ping}\" selected>"))
        );
        assert!(page.body.contains("value=\"120\""));

        // The switch's sensor page names the dependency; its device page inherits it.
        let detail = send(
            &h.app,
            get(&format!("/sensors/{switch_ping}"), Some(&admin)),
        )
        .await;
        assert!(
            detail.body.contains("abhängig von „Ping“ (Router)"),
            "{}",
            detail.body
        );
        assert!(
            detail
                .body
                .contains(&format!("/dependency/sensor/{switch_ping}"))
        );
        let page = send(
            &h.app,
            get(&format!("/dependency/device/{switch}"), Some(&viewer)),
        )
        .await;
        assert_eq!(page.status, StatusCode::FORBIDDEN);
        let page = send(
            &h.app,
            get(&format!("/dependency/device/{switch}"), Some(&admin)),
        )
        .await;
        assert!(
            page.body
                .contains("derzeit: „Ping“ (Router), von der Gruppe „Switche“"),
            "{}",
            page.body
        );
        assert!(page.body.contains("120 s, von der Gruppe „Switche“"));

        // The router ping depending on the switch ping would be a cycle.
        let reply = send(
            &h.app,
            post(
                &format!("/dependency/sensor/{router_ping}"),
                Some(&admin),
                &format!("csrf_token={csrf}&dependency=sensor&sensor={switch_ping}"),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(reply.body.contains("von sich selbst ab"), "{}", reply.body);

        // A master sensor is set on the sensor page.
        let sensor_path = format!("/dependency/sensor/{switch_ping}");
        let page = send(&h.app, get(&sensor_path, Some(&admin))).await;
        assert!(
            page.body.contains("name=\"master\"")
                && !page
                    .body
                    .contains(&format!("<option value=\"{switch_ping}\"")),
            "a sensor cannot depend on itself"
        );
        let reply = send(
            &h.app,
            post(
                &sensor_path,
                Some(&admin),
                &format!("csrf_token={csrf}&dependency=parent&master=on"),
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/sensors/{switch_ping}?msg=dependency_saved")
        );
        let detail = send(&h.app, get(reply.location(), Some(&admin))).await;
        assert!(detail.body.contains("Abhängigkeit gespeichert."));
        assert!(detail.body.contains("Master-Sensor des Geräts"));
    }
}
