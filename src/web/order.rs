//! "Reihenfolge ändern" (admin only): manual order of the subgroups and devices of a group (or of the
//! top level, id 0) and of the sensors of a device. The page works without JavaScript (position
//! numbers); `static/order.js` adds dragging and up/down buttons.

use std::collections::HashMap;

use askama::Template;
use axum::Form;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};

use crate::error::{Error, Result};
use crate::ordering::{positions_from_form, reorder};
use crate::storage::{devices, groups, sensors};
use crate::web::auth::CurrentUser;
use crate::web::{AppState, Layout};

struct Item {
    id: i64,
    name: String,
    sub: String,
    position: usize,
}

struct List {
    title: &'static str,
    /// Form field prefix: `g`, `d` or `s`.
    prefix: &'static str,
    items: Vec<Item>,
}

#[derive(Template)]
#[template(path = "order.html")]
struct OrderTemplate {
    layout: Layout,
    heading: String,
    /// Breadcrumb links before the last part: (href, label).
    crumbs: Vec<(String, String)>,
    back: String,
    action: String,
    lists: Vec<List>,
}

fn list(title: &'static str, prefix: &'static str, items: Vec<(i64, String, String)>) -> List {
    List {
        title,
        prefix,
        items: items
            .into_iter()
            .enumerate()
            .map(|(i, (id, name, sub))| Item {
                id,
                name,
                sub,
                position: i + 1,
            })
            .collect(),
    }
}

/// Top level (`0`) or an existing group.
async fn group_scope(state: &AppState, id: i64) -> Result<Option<crate::devices::Group>> {
    if id == 0 {
        return Ok(None);
    }
    groups::get_group(&state.pool, id)
        .await?
        .map(Some)
        .ok_or(Error::NotFound)
}

pub async fn group_page(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let group = group_scope(&state, id).await?;
    let parent = group.as_ref().map(|g| g.id);
    let subgroups = groups::list_groups(&state.pool)
        .await?
        .into_iter()
        .filter(|g| g.parent_id == parent)
        .map(|g| (g.id, g.name, String::new()))
        .collect();
    let members = devices::list_devices(&state.pool)
        .await?
        .into_iter()
        .filter(|d| d.group_id == parent)
        .map(|d| (d.id, d.name, d.host))
        .collect();
    let mut lists = vec![list(
        if parent.is_none() {
            "Gruppen"
        } else {
            "Untergruppen"
        },
        "g",
        subgroups,
    )];
    lists.push(list(
        if parent.is_none() {
            "Geräte ohne Gruppe"
        } else {
            "Geräte"
        },
        "d",
        members,
    ));
    lists.retain(|l| !l.items.is_empty());
    let page = OrderTemplate {
        layout: user.layout(),
        heading: match &group {
            Some(g) => format!("Reihenfolge in „{}“", g.name),
            None => "Reihenfolge der obersten Ebene".to_owned(),
        },
        crumbs: vec![("/".to_owned(), "Übersicht".to_owned())],
        back: "/".to_owned(),
        action: format!("/groups/{id}/order"),
        lists,
    };
    Ok(Html(page.render()?))
}

pub async fn sensor_page(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let device = devices::get_device(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)?;
    let mibs = state.mib_index();
    let items = sensors::list_sensors(&state.pool, id)
        .await?
        .into_iter()
        .map(|s| {
            let mut sub = crate::web::sensor_text::describe(&s.spec, &mibs);
            if !s.enabled {
                sub.push_str(" · inaktiv");
            }
            (s.id, s.name, sub)
        })
        .collect();
    let lists = vec![list("Sensoren", "s", items)];
    let page = OrderTemplate {
        layout: user.layout(),
        heading: format!("Reihenfolge der Sensoren von {}", device.name),
        crumbs: vec![
            ("/".to_owned(), "Übersicht".to_owned()),
            (format!("/devices/{id}/sensors"), device.name),
        ],
        back: format!("/devices/{id}/sensors"),
        action: format!("/devices/{id}/sensors/order"),
        lists,
    };
    Ok(Html(page.render()?))
}

/// `save` keeps the submitted order, `reset` returns to automatic order.
fn wants_reset(fields: &HashMap<String, String>) -> Result<bool> {
    match fields.get("action").map(String::as_str) {
        Some("save") => Ok(false),
        Some("reset") => Ok(true),
        _ => Err(Error::Invalid("Unbekannte Aktion.".to_owned())),
    }
}

pub async fn save_group(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(fields): Form<HashMap<String, String>>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(fields.get("csrf_token").map_or("", String::as_str))?;
    let parent = group_scope(&state, id).await?.map(|g| g.id);
    let reset = wants_reset(&fields)?;
    let (group_ids, device_ids) = if reset {
        (Vec::new(), Vec::new())
    } else {
        let current_groups: Vec<i64> = groups::list_groups(&state.pool)
            .await?
            .into_iter()
            .filter(|g| g.parent_id == parent)
            .map(|g| g.id)
            .collect();
        let current_devices: Vec<i64> = devices::list_devices(&state.pool)
            .await?
            .into_iter()
            .filter(|d| d.group_id == parent)
            .map(|d| d.id)
            .collect();
        (
            reorder(&current_groups, &positions_from_form(&fields, "g")),
            reorder(&current_devices, &positions_from_form(&fields, "d")),
        )
    };
    groups::set_positions(&state.pool, parent, &group_ids).await?;
    devices::set_positions(&state.pool, parent, &device_ids).await?;
    tracing::info!(user = %user.username, group = id, reset, "order of groups and devices saved");
    let msg = if reset { "order_reset" } else { "order_saved" };
    Ok(Redirect::to(&format!("/?msg={msg}")).into_response())
}

pub async fn save_sensors(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(fields): Form<HashMap<String, String>>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(fields.get("csrf_token").map_or("", String::as_str))?;
    devices::get_device(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)?;
    let reset = wants_reset(&fields)?;
    let ids = if reset {
        Vec::new()
    } else {
        let current: Vec<i64> = sensors::list_sensors(&state.pool, id)
            .await?
            .into_iter()
            .map(|s| s.id)
            .collect();
        reorder(&current, &positions_from_form(&fields, "s"))
    };
    sensors::set_positions(&state.pool, id, &ids).await?;
    tracing::info!(user = %user.username, device = id, reset, "order of sensors saved");
    let msg = if reset { "order_reset" } else { "order_saved" };
    Ok(Redirect::to(&format!("/devices/{id}/sensors?msg={msg}")).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{DeviceInput, Polling};
    use crate::sensors::spec::SensorSpec;
    use crate::storage::devices::save_device;
    use crate::storage::sensors::create_sensor;
    use crate::web::testutil::*;
    use axum::http::StatusCode;

    async fn device(h: &Harness, name: &str, group: Option<i64>, profile: i64) -> i64 {
        save_device(
            &h.state.pool,
            None,
            DeviceInput {
                name: name.to_owned(),
                host: "192.0.2.1".to_owned(),
                group_id: group,
                profile_id: Some(profile),
                polling: Polling::default(),
            },
        )
        .await
        .unwrap()
    }

    async fn sensor_names(h: &Harness, device: i64) -> Vec<String> {
        sensors::list_sensors(&h.state.pool, device)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect()
    }

    #[tokio::test]
    async fn sensors_are_reordered_by_position_and_reset() {
        let h = harness(true).await;
        let profile = add_v2c_profile(&h, "P", "public").await;
        let dev = device(&h, "Switch", None, profile).await;
        let other = device(&h, "Anderes", None, profile).await;
        let ping = create_sensor(&h.state.pool, dev, "Ping", &SensorSpec::Ping)
            .await
            .unwrap();
        let up = create_sensor(&h.state.pool, dev, "Uptime", &SensorSpec::Uptime)
            .await
            .unwrap();
        let t2 = create_sensor(
            &h.state.pool,
            dev,
            "Traffic Port 2",
            &SensorSpec::InterfaceTraffic { if_index: 2 },
        )
        .await
        .unwrap();
        let foreign = create_sensor(&h.state.pool, other, "Ping", &SensorSpec::Ping)
            .await
            .unwrap();
        let (admin, csrf) = login(&h, "anna").await;

        let path = format!("/devices/{dev}/sensors/order");
        let page = send(&h.app, get(&path, Some(&admin))).await;
        assert_eq!(page.status, StatusCode::OK);
        for needle in [
            "name=\"s_1\" value=\"1\"",
            "name=\"s_3\" value=\"3\"",
            "/static/order.js",
            "data-move=\"up\"",
            "Reihenfolge der Sensoren von Switch",
        ] {
            assert!(page.body.contains(needle), "{needle}: {}", page.body);
        }

        let reply = send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!(
                    "csrf_token={csrf}&action=save&s_{t2}=1&s_{ping}=2&s_{up}=3&s_{foreign}=1"
                ),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        assert!(reply.location().ends_with("msg=order_saved"));
        assert_eq!(
            sensor_names(&h, dev).await,
            ["Traffic Port 2", "Ping", "Uptime"]
        );
        assert_eq!(sensor_names(&h, other).await, ["Ping"]);
        let home = send(&h.app, get("/", Some(&admin))).await;
        assert!(
            !home.body.contains("Gruppen anordnen"),
            "no groups, no button"
        );
        let list = send(
            &h.app,
            get(&format!("/devices/{dev}/sensors"), Some(&admin)),
        )
        .await;
        let (a, b) = (
            list.body.find("Traffic Port 2").unwrap(),
            list.body.find(">Ping<").unwrap(),
        );
        assert!(a < b, "the sensor page shows the saved order");

        let reply = send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!("csrf_token={csrf}&action=reset"),
            ),
        )
        .await;
        assert!(reply.location().ends_with("msg=order_reset"));
        assert_eq!(
            sensor_names(&h, dev).await,
            ["Ping", "Uptime", "Traffic Port 2"]
        );

        assert_eq!(
            send(
                &h.app,
                post(
                    &path,
                    Some(&admin),
                    &format!("csrf_token={csrf}&action=boom")
                )
            )
            .await
            .status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[tokio::test]
    async fn groups_and_devices_are_reordered_per_level() {
        let h = harness(true).await;
        let profile = add_v2c_profile(&h, "P", "public").await;
        let haus = groups::create_group(&h.state.pool, "Haus", None)
            .await
            .unwrap();
        let keller = groups::create_group(&h.state.pool, "Keller", None)
            .await
            .unwrap();
        let sw = device(&h, "Switch", Some(haus), profile).await;
        let ap = device(&h, "AP", Some(haus), profile).await;
        let loose = device(&h, "Router", None, profile).await;
        let (admin, csrf) = login(&h, "anna").await;

        let page = send(&h.app, get("/groups/0/order", Some(&admin))).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains("Gruppen") && page.body.contains("Geräte ohne Gruppe"));
        assert!(page.body.contains(&format!("name=\"g_{keller}\"")));
        assert!(page.body.contains(&format!("name=\"d_{loose}\"")));
        assert!(
            !page.body.contains(&format!("name=\"d_{sw}\"")),
            "only this level"
        );

        let reply = send(
            &h.app,
            post(
                "/groups/0/order",
                Some(&admin),
                &format!("csrf_token={csrf}&action=save&g_{keller}=1&g_{haus}=2&d_{loose}=1"),
            ),
        )
        .await;
        let home = send(&h.app, get(reply.location(), Some(&admin))).await;
        assert!(home.body.find("Keller").unwrap() < home.body.find("Haus").unwrap());
        assert!(home.body.contains("Reihenfolge gespeichert."));
        assert!(
            home.body
                .contains("href=\"/groups/0/order\">Gruppen anordnen</a>")
        );

        let page = send(&h.app, get(&format!("/groups/{haus}/order"), Some(&admin))).await;
        assert!(page.body.contains("Reihenfolge in „Haus“") && !page.body.contains("Untergruppen"));
        send(
            &h.app,
            post(
                &format!("/groups/{haus}/order"),
                Some(&admin),
                &format!("csrf_token={csrf}&action=save&d_{sw}=1&d_{ap}=2"),
            ),
        )
        .await;
        let names: Vec<String> = devices::list_devices(&h.state.pool)
            .await
            .unwrap()
            .into_iter()
            .filter(|d| d.group_id == Some(haus))
            .map(|d| d.name)
            .collect();
        assert_eq!(names, ["Switch", "AP"]);

        assert_eq!(
            send(&h.app, get("/groups/999/order", Some(&admin)))
                .await
                .status,
            StatusCode::NOT_FOUND
        );
        let (viewer, vcsrf) = login(&h, "bert").await;
        for (method, path) in [
            ("get", "/groups/0/order".to_owned()),
            ("post", format!("/devices/{sw}/sensors/order")),
        ] {
            let request = if method == "get" {
                get(&path, Some(&viewer))
            } else {
                post(
                    &path,
                    Some(&viewer),
                    &format!("csrf_token={vcsrf}&action=reset"),
                )
            };
            assert_eq!(
                send(&h.app, request).await.status,
                StatusCode::FORBIDDEN,
                "{path}"
            );
        }
    }
}
