//! Pausing and resuming groups, devices and sensors (admin only): `/pause/<kind>/<id>` asks for the
//! duration and an optional reason, `/resume/<kind>/<id>` ends the pause.

use askama::Template;
use axum::Form;
use axum::extract::{Path, Query, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::error::{Error, Result};
use crate::i18n::{Lang, Msg};
use crate::pause::{DURATIONS_MIN, Pause, Target, Why, parse_duration, validate_reason};
use crate::storage::{devices, groups, pauses, sensors};
use crate::web::auth::CurrentUser;
use crate::web::{AppState, Crumb, Layout, back_to_group, group_trail, time_html};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// How a pause is shown: "Pausiert mit dem Gerät bis <time> · Grund".
pub struct PauseNote {
    pub text: String,
    /// The end as `<time>` HTML (`None` = until resumed).
    pub until: Option<String>,
    pub reason: String,
}

pub fn note(t: Lang, why: &Why) -> PauseNote {
    PauseNote {
        text: t.msg(&why.label()),
        until: why.pause.until_ms.map(time_html),
        reason: why.pause.reason.clone(),
    }
}

/// Where the pause pages return to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Back {
    Overview,
    /// The sensor page of the device.
    Device,
    /// The detail page of the sensor.
    Sensor,
    /// The page of a group (`group-<id>`).
    Group(i64),
}

impl Back {
    fn parse(s: &str, target: Target) -> Self {
        if let Some(id) = back_to_group(s) {
            return Self::Group(id);
        }
        match (s, target) {
            ("overview", _) => Self::Overview,
            ("sensor", Target::Sensor(_)) => Self::Sensor,
            ("device", Target::Sensor(_) | Target::Device(_)) => Self::Device,
            (_, Target::Group(_)) => Self::Overview,
            _ => Self::Device,
        }
    }

    fn as_string(self) -> String {
        match self {
            Self::Overview => "overview".to_owned(),
            Self::Device => "device".to_owned(),
            Self::Sensor => "sensor".to_owned(),
            Self::Group(id) => format!("group-{id}"),
        }
    }
}

/// The paused object with what the pages need of it (also for the dependency page).
pub(super) struct Object {
    pub(super) target: Target,
    pub(super) name: String,
    /// Device of a device or sensor.
    pub(super) device_id: Option<i64>,
    /// Übersicht › groups › device › sensor, down to the object itself.
    pub(super) trail: Vec<Crumb>,
}

impl Object {
    fn href(&self, back: Back, msg: Option<&str>) -> String {
        let query = msg.map(|m| format!("?msg={m}")).unwrap_or_default();
        match (back, self.target, self.device_id) {
            (Back::Sensor, Target::Sensor(id), _) => format!("/sensors/{id}{query}"),
            (Back::Device, _, Some(device)) => format!("/devices/{device}/sensors{query}"),
            (Back::Group(group), ..) => format!("/groups/{group}{query}"),
            (_, Target::Group(id), _) => format!("/{query}#group-{id}"),
            _ => format!("/{query}"),
        }
    }
}

pub(super) async fn object(state: &AppState, t: Lang, target: Target) -> Result<Object> {
    let overview = Crumb {
        name: t.get("nav-overview"),
        href: "/".to_owned(),
    };
    let device_crumbs = |device: &crate::devices::Device, trail: Vec<Crumb>| {
        let mut crumbs = vec![overview.clone()];
        crumbs.extend(trail);
        crumbs.push(Crumb {
            name: device.name.clone(),
            href: format!("/devices/{}/sensors", device.id),
        });
        crumbs
    };
    match target {
        Target::Sensor(id) => {
            let sensor = sensors::get_sensor(&state.pool, id)
                .await?
                .ok_or(Error::NotFound)?;
            let device = devices::get_device(&state.pool, sensor.device_id)
                .await?
                .ok_or(Error::NotFound)?;
            let mut trail =
                device_crumbs(&device, group_trail(&state.pool, device.group_id).await?);
            trail.push(Crumb {
                name: sensor.name.clone(),
                href: format!("/sensors/{id}"),
            });
            Ok(Object {
                target,
                name: sensor.name,
                device_id: Some(device.id),
                trail,
            })
        }
        Target::Device(id) => {
            let device = devices::get_device(&state.pool, id)
                .await?
                .ok_or(Error::NotFound)?;
            let trail = device_crumbs(&device, group_trail(&state.pool, device.group_id).await?);
            Ok(Object {
                target,
                name: device.name,
                device_id: Some(id),
                trail,
            })
        }
        Target::Group(id) => {
            let group = groups::list_groups(&state.pool)
                .await?
                .into_iter()
                .find(|g| g.id == id)
                .ok_or(Error::NotFound)?;
            let mut trail = vec![overview];
            trail.extend(group_trail(&state.pool, Some(id)).await?);
            Ok(Object {
                target,
                name: group.name,
                device_id: None,
                trail,
            })
        }
    }
}

fn target(kind: &str, id: i64) -> Result<Target> {
    Target::parse(kind, id).ok_or(Error::NotFound)
}

struct DurationOption {
    value: u32,
    label: String,
    selected: bool,
}

#[derive(Template)]
#[template(path = "pause_form.html")]
struct PauseTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    heading: String,
    /// What is paused with the object, or empty for a sensor.
    hint: String,
    trail: Vec<Crumb>,
    action: String,
    back: String,
    cancel_href: String,
    durations: Vec<DurationOption>,
    reason: String,
    error: Option<String>,
}

fn duration_label(t: Lang, minutes: u32) -> String {
    match minutes {
        0 => t.get("pause-until-resumed"),
        m if m % 1440 == 0 => t.get1("pause-days", "count", m / 1440),
        m if m % 60 == 0 => t.get1("pause-hours", "count", m / 60),
        m => t.get1("pause-minutes", "count", m),
    }
}

fn render(
    user: &CurrentUser,
    obj: &Object,
    back: Back,
    duration: u32,
    reason: String,
    error: Option<String>,
) -> Result<String> {
    let t = user.lang;
    let page = PauseTemplate {
        layout: user.layout(),
        heading: t.get1("pause-heading", "name", obj.name.as_str()),
        hint: match obj.target {
            Target::Group(_) => t.get("pause-hint-group"),
            Target::Device(_) => t.get("pause-hint-device"),
            Target::Sensor(_) => String::new(),
        },
        trail: obj.trail.clone(),
        action: format!("/pause/{}/{}", obj.target.kind(), obj.target.id()),
        back: back.as_string(),
        cancel_href: obj.href(back, None),
        durations: DURATIONS_MIN
            .into_iter()
            .map(|m| DurationOption {
                value: m,
                label: duration_label(t, m),
                selected: m == duration,
            })
            .collect(),
        reason,
        error,
        t,
    };
    Ok(page.render()?)
}

#[derive(Deserialize)]
pub struct BackQuery {
    #[serde(default)]
    back: String,
}

pub async fn form(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((kind, id)): Path<(String, i64)>,
    Query(query): Query<BackQuery>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let target = target(&kind, id)?;
    let obj = object(&state, user.lang, target).await?;
    let back = Back::parse(&query.back, target);
    Ok(Html(render(&user, &obj, back, 0, String::new(), None)?))
}

/// Deliberately no `Debug`, consistent with the other forms.
#[derive(Deserialize)]
pub struct PauseForm {
    csrf_token: String,
    #[serde(default)]
    duration: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    back: String,
}

fn flash(target: Target, paused: bool) -> &'static str {
    match (target, paused) {
        (Target::Sensor(_), true) => "sensor_paused",
        (Target::Sensor(_), false) => "sensor_resumed",
        (Target::Device(_), true) => "device_paused",
        (Target::Device(_), false) => "device_resumed",
        (Target::Group(_), true) => "group_paused",
        (Target::Group(_), false) => "group_resumed",
    }
}

pub async fn pause(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((kind, id)): Path<(String, i64)>,
    Form(form): Form<PauseForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let target = target(&kind, id)?;
    let obj = object(&state, user.lang, target).await?;
    let back = Back::parse(&form.back, target);
    let now = now_ms();
    let checked = parse_duration(&form.duration).and_then(|minutes| {
        Ok(Pause {
            until_ms: minutes.map(|m| now + i64::from(m) * 60_000),
            reason: validate_reason(&form.reason)?,
        })
    });
    let pause = match checked {
        Ok(pause) => pause,
        Err(msg) => {
            let duration = form.duration.trim().parse().unwrap_or(0);
            let page = render(
                &user,
                &obj,
                back,
                duration,
                form.reason.clone(),
                Some(user.lang.msg(&msg)),
            )?;
            return Ok((axum::http::StatusCode::UNPROCESSABLE_ENTITY, Html(page)).into_response());
        }
    };
    pauses::pause(&state.pool, target, &pause, &user.username, now).await?;
    tracing::info!(user = %user.username, kind, id, until = ?pause.until_ms, "paused");
    Ok(Redirect::to(&obj.href(back, Some(flash(target, true)))).into_response())
}

#[derive(Deserialize)]
pub struct ResumeForm {
    csrf_token: String,
    #[serde(default)]
    back: String,
}

pub async fn resume(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((kind, id)): Path<(String, i64)>,
    Form(form): Form<ResumeForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let target = target(&kind, id)?;
    let obj = object(&state, user.lang, target).await?;
    if !pauses::resume(&state.pool, target, &user.username, now_ms()).await? {
        return Err(Error::invalid(Msg::new("pause-not-paused")));
    }
    tracing::info!(user = %user.username, kind, id, "resumed");
    let back = Back::parse(&form.back, target);
    Ok(Redirect::to(&obj.href(back, Some(flash(target, false)))).into_response())
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;
    use crate::devices::{DeviceInput, Polling};
    use crate::sensors::spec::SensorSpec;
    use crate::storage::devices::save_device;
    use crate::web::testutil::*;

    /// Group "Netzwerk" › ping-only device "Router" with a ping sensor: (group, device, sensor).
    async fn tree(h: &Harness) -> (i64, i64, i64) {
        let group = groups::create_group(&h.state.pool, "Netzwerk", None)
            .await
            .unwrap();
        let device = save_device(
            &h.state.pool,
            None,
            DeviceInput {
                kind: Default::default(),
                name: "Router".to_owned(),
                host: "192.0.2.1".to_owned(),
                group_id: Some(group),
                profile_id: None,
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
        let sensor = sensors::create_sensor(&h.state.pool, device, "Ping", &SensorSpec::Ping)
            .await
            .unwrap();
        (group, device, sensor)
    }

    #[tokio::test]
    async fn a_paused_group_pauses_its_devices_and_everything_is_logged() {
        let h = harness(true).await;
        let (group, device, sensor) = tree(&h).await;
        let (viewer, vcsrf) = login(&h, "bert").await;
        let (admin, csrf) = login(&h, "anna").await;
        let form_path = format!("/pause/group/{group}");

        assert_eq!(
            send(&h.app, get(&form_path, Some(&viewer))).await.status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(
                &h.app,
                post(
                    &form_path,
                    Some(&viewer),
                    &format!("csrf_token={vcsrf}&duration=0")
                )
            )
            .await
            .status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(&h.app, get("/pause/probe/1", Some(&admin)))
                .await
                .status,
            StatusCode::NOT_FOUND
        );

        // A polled sensor: pausing ends its state (one never polled has none to end).
        crate::storage::status::record_poll(
            &h.state.pool,
            sensor,
            1_000,
            &[],
            &crate::status::Assessment::Healthy(Msg::Text("ok".into())),
        )
        .await
        .unwrap();
        let page = send(&h.app, get(&form_path, Some(&admin))).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains("„Netzwerk“ pausieren"), "{}", page.body);
        assert!(page.body.contains("Alle Untergruppen, Geräte und Sensoren"));
        let page = send(
            &h.app,
            post(
                &form_path,
                Some(&admin),
                &format!("csrf_token={csrf}&duration=7&reason=Umbau"),
            ),
        )
        .await;
        assert_eq!(page.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            page.body.contains("Bitte eine der angebotenen Dauern") && page.body.contains("Umbau")
        );

        let reply = send(
            &h.app,
            post(
                &form_path,
                Some(&admin),
                &format!("csrf_token={csrf}&duration=1440&reason=Umbau&back=overview"),
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/?msg=group_paused#group-{group}")
        );
        let status: String =
            sqlx::query_scalar("SELECT status FROM sensor_status WHERE sensor_id = ?")
                .bind(sensor)
                .fetch_one(&h.state.pool)
                .await
                .unwrap();
        assert_eq!(status, "paused");

        let home = send(&h.app, get("/", Some(&admin))).await;
        assert!(
            home.body.contains("Umbau") && home.body.contains(&format!("/resume/group/{group}"))
        );
        let page = send(
            &h.app,
            get(&format!("/devices/{device}/sensors"), Some(&admin)),
        )
        .await;
        assert!(
            page.body.contains("Pausiert mit der Gruppe „Netzwerk“"),
            "{}",
            page.body
        );
        let detail = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&admin))).await;
        assert!(detail.body.contains("Pausiert mit der Gruppe „Netzwerk“"));

        // Only the group itself can be resumed, not the device paused with it.
        assert_eq!(
            send(
                &h.app,
                post(
                    &format!("/resume/device/{device}"),
                    Some(&admin),
                    &format!("csrf_token={csrf}")
                )
            )
            .await
            .status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let reply = send(
            &h.app,
            post(
                &format!("/resume/group/{group}"),
                Some(&admin),
                &format!("csrf_token={csrf}&back=overview"),
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/?msg=group_resumed#group-{group}")
        );

        // A timed device pause that runs out.
        pauses::pause(
            &h.state.pool,
            Target::Device(device),
            &Pause {
                until_ms: Some(now_ms() - 1_000),
                reason: String::new(),
            },
            "anna",
            now_ms() - 60_000,
        )
        .await
        .unwrap();
        assert_eq!(pauses::expire(&h.state.pool, now_ms()).await.unwrap(), 1);

        let log = send(&h.app, get("/log", Some(&viewer))).await;
        assert_eq!(log.status, StatusCode::OK);
        for text in [
            "Fortgesetzt",
            "von anna · Umbau",
            "ganze Gruppe",
            "ganzes Gerät",
            "Pause abgelaufen",
            "automatisch fortgesetzt",
            "Netzwerk",
            "Router",
        ] {
            assert!(log.body.contains(text), "{text}: {}", log.body);
        }
        assert!(log.body.contains("badge status-paused"));
    }
}
