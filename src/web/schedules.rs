//! Maintenance windows (admin only): the schedules `/schedules` with their windows, and which
//! schedule applies to a group, device or sensor (`/maintenance/<kind>/<id>`: inherit, a schedule or
//! none). Times on these pages are in the time zone of the server, like the windows themselves.
//! See [`crate::schedule`].

use askama::Template;
use axum::Form;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::audit::{Action, Changes, Kind};
use crate::error::{Error, Result};
use crate::i18n::{Lang, Msg};
use crate::pause::{Origin, Pauses, Target};
use crate::schedule::{self, Assignment, EVERY_DAY, Window};
use crate::storage::pauses;
use crate::storage::schedules::{self as store, Schedule};
use crate::web::audit::{self, Object as AuditObject};
use crate::web::auth::{CsrfForm, CurrentUser};
use crate::web::pause::{Object, object};
use crate::web::sensors::now_ms;
use crate::web::{AppState, Crumb, Layout, flash_message};

#[derive(Deserialize)]
pub struct FlashQuery {
    msg: Option<String>,
}

const WEEKDAY_IDS: [&str; 7] = [
    "weekday-1",
    "weekday-2",
    "weekday-3",
    "weekday-4",
    "weekday-5",
    "weekday-6",
    "weekday-7",
];
const WORKDAYS: u8 = 0b001_1111;
const WEEKEND: u8 = 0b110_0000;

/// A time in the zone of the schedules, e.g. `2026-10-31 10:00`.
fn local(ms: i64) -> String {
    schedule::format_local(ms, schedule::time_zone()).replace('T', " ")
}

fn days(mask: u8) -> Msg {
    match mask {
        EVERY_DAY => Msg::new("schedule-every-day"),
        WORKDAYS => Msg::new("schedule-workdays"),
        WEEKEND => Msg::new("schedule-weekend"),
        _ => Msg::List(
            WEEKDAY_IDS
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, id)| Msg::new(id))
                .collect(),
        ),
    }
}

/// "15 Min.", "1 Std. 30 Min." or "2 T. 0 Std. 0 Min." (as `snmp::format_uptime`).
fn duration(minutes: u32) -> Msg {
    let (days, hours, minutes) = (minutes / 1_440, (minutes % 1_440) / 60, minutes % 60);
    match (days, hours) {
        (0, 0) => Msg::new("duration-minutes").with("minutes", minutes),
        (0, _) => Msg::new("duration-hours-minutes")
            .with("hours", hours)
            .with("minutes", minutes),
        _ => Msg::new("duration-days-hours-minutes")
            .with("days", days)
            .with("hours", hours)
            .with("minutes", minutes),
    }
}

/// "Mo–Fr ab 03:00 Uhr, 15 Min." or "2026-10-31 10:00 bis 2026-10-31 11:00".
pub fn window_msg(w: &Window) -> Msg {
    match *w {
        Window::Weekly {
            weekdays,
            start_minute,
            duration_minutes,
        } => Msg::new("schedule-weekly")
            .with("days", days(weekdays))
            .with("time", schedule::format_time(start_minute))
            .with("duration", duration(duration_minutes)),
        Window::Once { start_ms, end_ms } => Msg::new("schedule-once")
            .with("start", local(start_ms))
            .with("end", local(end_ms)),
    }
}

pub fn describe_window(t: Lang, w: &Window) -> String {
    t.msg(&window_msg(w))
}

/// "offen bis …", "nächster Beginn …" or "kein weiteres Fenster".
fn state_text(t: Lang, windows: &[Window], now: i64) -> (String, bool) {
    let tz = schedule::time_zone();
    if let Some(until) = schedule::active_until(windows, now, tz) {
        return (
            t.get1("schedule-open-until", "time", local(until).as_str()),
            true,
        );
    }
    match schedule::next_start(windows, now, tz) {
        Some(next) => (t.get1("schedule-next", "time", local(next).as_str()), false),
        None => (t.get("schedule-no-next"), false),
    }
}

struct Row {
    id: i64,
    name: String,
    windows: String,
    state: String,
    open: bool,
    used: i64,
}

#[derive(Template)]
#[template(path = "schedules.html")]
struct ListTemplate {
    t: Lang,
    layout: Layout,
    zone: String,
    rows: Vec<Row>,
    name: String,
    flash: Option<String>,
    error: Option<String>,
}

async fn list_page(
    state: &AppState,
    user: &CurrentUser,
    name: String,
    flash: Option<String>,
    error: Option<String>,
) -> Result<String> {
    let t = user.lang;
    let now = now_ms();
    let rows = store::list(&state.pool)
        .await?
        .into_iter()
        .map(|s| {
            let windows = s.plain_windows();
            let (state, open) = state_text(t, &windows, now);
            Row {
                id: s.id,
                windows: if windows.is_empty() {
                    t.get("schedule-no-windows")
                } else {
                    windows
                        .iter()
                        .map(|w| describe_window(t, w))
                        .collect::<Vec<_>>()
                        .join("; ")
                },
                state,
                open,
                used: s.used,
                name: s.name,
            }
        })
        .collect();
    let page = ListTemplate {
        layout: user.layout(),
        zone: schedule::zone_name(schedule::time_zone()),
        rows,
        name,
        flash,
        error,
        t,
    };
    Ok(page.render()?)
}

pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<FlashQuery>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let flash = flash_message(user.lang, query.msg.as_deref());
    Ok(Html(
        list_page(&state, &user, String::new(), flash, None).await?,
    ))
}

#[derive(Deserialize)]
pub struct NameForm {
    csrf_token: String,
    #[serde(default)]
    name: String,
}

pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Form(form): Form<NameForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let result = match schedule::validate_name(&form.name) {
        Ok(name) => store::create(&state.pool, &name).await.map(|id| (id, name)),
        Err(msg) => Err(Error::Invalid(msg)),
    };
    match result {
        Ok((id, name)) => {
            tracing::info!(user = %user.username, schedule = id, "schedule created");
            let object = AuditObject::new(Kind::Schedule, id, &name);
            audit::record(&state, &user, Action::Created, object, Changes::new()).await;
            Ok(Redirect::to(&format!("/schedules/{id}?msg=schedule_created")).into_response())
        }
        Err(Error::Invalid(msg) | Error::Conflict(msg)) => {
            let error = Some(user.lang.msg(&msg));
            let html = list_page(&state, &user, form.name, None, error).await?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response())
        }
        Err(e) => Err(e),
    }
}

struct WindowRow {
    id: i64,
    text: String,
    state: String,
    open: bool,
}

/// What the "add window" forms show again after an error.
#[derive(Default)]
struct WindowValues {
    weekdays: u8,
    time: String,
    duration: String,
    start: String,
    end: String,
}

#[derive(Template)]
#[template(path = "schedule_detail.html")]
struct DetailTemplate {
    t: Lang,
    layout: Layout,
    zone: String,
    schedule_id: i64,
    schedule_name: String,
    used: i64,
    state: String,
    open: bool,
    windows: Vec<WindowRow>,
    /// (value 1–7, label, checked)
    weekdays: Vec<(u8, String, bool)>,
    values: WindowValues,
    name: String,
    flash: Option<String>,
    error: Option<String>,
}

async fn load(state: &AppState, id: i64) -> Result<Schedule> {
    store::get(&state.pool, id).await?.ok_or(Error::NotFound)
}

fn detail_page(
    user: &CurrentUser,
    s: Schedule,
    values: WindowValues,
    name: Option<String>,
    flash: Option<String>,
    error: Option<String>,
) -> Result<String> {
    let t = user.lang;
    let now = now_ms();
    let (state, open) = state_text(t, &s.plain_windows(), now);
    let windows = s
        .windows
        .iter()
        .map(|w| {
            let (state, open) = if w.window.is_over(now) {
                (t.get("schedule-window-over"), false)
            } else {
                state_text(t, &[w.window], now)
            };
            WindowRow {
                id: w.id,
                text: describe_window(t, &w.window),
                state,
                open,
            }
        })
        .collect();
    let checked = if values.weekdays == 0 {
        WORKDAYS
    } else {
        values.weekdays
    };
    let page = DetailTemplate {
        layout: user.layout(),
        zone: schedule::zone_name(schedule::time_zone()),
        schedule_id: s.id,
        name: name.unwrap_or_else(|| s.name.clone()),
        schedule_name: s.name,
        used: s.used,
        state,
        open,
        windows,
        weekdays: WEEKDAY_IDS
            .iter()
            .enumerate()
            .map(|(i, id)| (i as u8 + 1, t.get(id), checked & (1 << i) != 0))
            .collect(),
        values: WindowValues {
            time: if values.time.is_empty() {
                "03:00".to_owned()
            } else {
                values.time
            },
            duration: if values.duration.is_empty() {
                "15".to_owned()
            } else {
                values.duration
            },
            ..values
        },
        flash,
        error,
        t,
    };
    Ok(page.render()?)
}

pub async fn detail(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<FlashQuery>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let s = load(&state, id).await?;
    let flash = flash_message(user.lang, query.msg.as_deref());
    Ok(Html(detail_page(
        &user,
        s,
        WindowValues::default(),
        None,
        flash,
        None,
    )?))
}

pub async fn rename(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<NameForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let s = load(&state, id).await?;
    let result = match schedule::validate_name(&form.name) {
        Ok(name) => store::rename(&state.pool, id, &name).await.map(|()| name),
        Err(msg) => Err(Error::Invalid(msg)),
    };
    match result {
        Ok(name) => {
            tracing::info!(user = %user.username, schedule = id, "schedule renamed");
            let changes = Changes::new().field("audit-field-name", &s.name, &name);
            let object = AuditObject::new(Kind::Schedule, id, &name);
            audit::record(&state, &user, Action::Changed, object, changes).await;
            Ok(Redirect::to(&format!("/schedules/{id}?msg=schedule_saved")).into_response())
        }
        Err(Error::Invalid(msg) | Error::Conflict(msg)) => {
            let error = Some(user.lang.msg(&msg));
            let html = detail_page(
                &user,
                s,
                WindowValues::default(),
                Some(form.name),
                None,
                error,
            )?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response())
        }
        Err(e) => Err(e),
    }
}

pub async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let s = load(&state, id).await?;
    if !store::delete(&state.pool, id).await? {
        return Err(Error::NotFound);
    }
    tracing::info!(user = %user.username, schedule = id, "schedule deleted");
    let object = AuditObject::new(Kind::Schedule, id, &s.name);
    audit::record(&state, &user, Action::Deleted, object, Changes::new()).await;
    // Objects in one of its windows are not paused any more.
    pauses::sync_all(&state.pool, now_ms()).await?;
    Ok(Redirect::to("/schedules?msg=schedule_deleted").into_response())
}

/// The fields of the "add window" forms; read as pairs because the weekday checkbox `weekday`
/// (1 = Monday … 7 = Sunday) repeats.
struct WindowForm {
    kind: String,
    weekdays: Vec<String>,
    time: String,
    duration: String,
    start: String,
    end: String,
}

impl WindowForm {
    fn from_pairs(pairs: &[(String, String)]) -> Self {
        let field = |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        Self {
            kind: field("kind"),
            weekdays: pairs
                .iter()
                .filter(|(k, _)| k == "weekday")
                .map(|(_, v)| v.clone())
                .collect(),
            time: field("time"),
            duration: field("duration"),
            start: field("start"),
            end: field("end"),
        }
    }
}

pub async fn add_window(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Result<Response> {
    user.require_admin()?;
    let csrf = pairs
        .iter()
        .find(|(k, _)| k == "csrf_token")
        .map_or("", |(_, v)| v.as_str());
    user.verify_csrf(csrf)?;
    let form = WindowForm::from_pairs(&pairs);
    let s = load(&state, id).await?;
    let tz = schedule::time_zone();
    let days = &form.weekdays;
    let parsed = match form.kind.as_str() {
        "weekly" => schedule::parse_weekdays(days).and_then(|weekdays| {
            Ok(Window::Weekly {
                weekdays,
                start_minute: schedule::parse_time(&form.time)?,
                duration_minutes: schedule::parse_duration(&form.duration)?,
            })
        }),
        "once" => schedule::parse_once(&form.start, &form.end, tz),
        _ => {
            return Err(Error::invalid(crate::i18n::Msg::new(
                "schedule-kind-invalid",
            )));
        }
    };
    match parsed {
        Ok(window) => {
            store::add_window(&state.pool, id, window).await?;
            tracing::info!(user = %user.username, schedule = id, ?window, "maintenance window added");
            let changes = Changes::new()
                .note(Msg::new("audit-window-added").with("window", window_msg(&window)));
            let object = AuditObject::new(Kind::Schedule, id, &s.name);
            audit::record(&state, &user, Action::Changed, object, changes).await;
            pauses::sync_all(&state.pool, now_ms()).await?;
            Ok(Redirect::to(&format!("/schedules/{id}?msg=window_added")).into_response())
        }
        Err(msg) => {
            let values = WindowValues {
                weekdays: schedule::parse_weekdays(days).unwrap_or(0),
                time: form.time,
                duration: form.duration,
                start: form.start,
                end: form.end,
            };
            let error = Some(user.lang.msg(&msg));
            let html = detail_page(&user, s, values, None, None, error)?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response())
        }
    }
}

pub async fn delete_window(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, window)): Path<(i64, i64)>,
    Form(form): Form<CsrfForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let s = load(&state, id).await?;
    let removed = s.windows.iter().find(|w| w.id == window).map(|w| w.window);
    if !store::delete_window(&state.pool, id, window).await? {
        return Err(Error::NotFound);
    }
    tracing::info!(user = %user.username, schedule = id, window, "maintenance window deleted");
    if let Some(removed) = removed {
        let changes = Changes::new()
            .note(Msg::new("audit-window-deleted").with("window", window_msg(&removed)));
        let object = AuditObject::new(Kind::Schedule, id, &s.name);
        audit::record(&state, &user, Action::Changed, object, changes).await;
    }
    pauses::sync_all(&state.pool, now_ms()).await?;
    Ok(Redirect::to(&format!("/schedules/{id}?msg=window_deleted")).into_response())
}

// ---------------------------------------------------------------------------------------------
// Which schedule applies to an object

struct ScheduleOption {
    id: i64,
    name: String,
    selected: bool,
}

#[derive(Template)]
#[template(path = "maintenance_form.html")]
struct AssignTemplate {
    t: Lang,
    layout: Layout,
    heading: String,
    trail: Vec<Crumb>,
    action: String,
    cancel_href: String,
    /// `parent`, `schedule` or `none`.
    choice: String,
    inherited: String,
    /// What applies now, with its next window.
    current: String,
    schedules: Vec<ScheduleOption>,
    zone: String,
    error: Option<String>,
}

fn href(obj: &Object, msg: Option<&str>) -> String {
    let query = msg.map(|m| format!("?msg={m}")).unwrap_or_default();
    match obj.target {
        Target::Group(id) => format!("/groups/{id}{query}"),
        Target::Device(id) => format!("/devices/{id}/sensors{query}"),
        Target::Sensor(id) => format!("/sensors/{id}{query}"),
    }
}

/// "„Nachts“ (von der Gruppe „Netzwerk“) · nächster Beginn …" or "keins".
fn describe(
    t: Lang,
    pauses: &Pauses,
    (id, from): (Option<i64>, Origin),
    list: &[Schedule],
) -> String {
    let Some(s) = id.and_then(|id| list.iter().find(|s| s.id == id)) else {
        return t.get("maintenance-none-applies");
    };
    let mut text = t.get1("maintenance-schedule-named", "name", s.name.as_str());
    match from {
        Origin::Device => text = format!("{text}, {}", t.get("dependency-from-device")),
        Origin::Group(name) => {
            text = format!(
                "{text}, {}",
                t.get1("dependency-from-group", "name", name.as_str())
            );
        }
        Origin::Own | Origin::Default => {}
    }
    let open = pauses.schedule_info(s.id).and_then(|i| i.open_until);
    let (state, _) = match open {
        Some(until) => (
            t.get1("schedule-open-until", "time", local(until).as_str()),
            true,
        ),
        None => state_text(t, &s.plain_windows(), now_ms()),
    };
    format!("{text} · {state}")
}

async fn assign_page(
    state: &AppState,
    user: &CurrentUser,
    obj: &Object,
    choice: String,
    selected: Option<i64>,
    error: Option<String>,
) -> Result<String> {
    let t = user.lang;
    let pauses = pauses::load_all(&state.pool, now_ms()).await?;
    let list = store::list(&state.pool).await?;
    let page = AssignTemplate {
        layout: user.layout(),
        heading: t.get1("maintenance-heading", "name", obj.name.as_str()),
        trail: obj.trail.clone(),
        action: format!("/maintenance/{}/{}", obj.target.kind(), obj.target.id()),
        cancel_href: href(obj, None),
        choice,
        inherited: describe(t, &pauses, pauses.schedule_inherited(obj.target), &list),
        current: describe(t, &pauses, pauses.schedule_of(obj.target), &list),
        schedules: list
            .iter()
            .map(|s| ScheduleOption {
                id: s.id,
                name: s.name.clone(),
                selected: selected == Some(s.id),
            })
            .collect(),
        zone: schedule::zone_name(schedule::time_zone()),
        error,
        t,
    };
    Ok(page.render()?)
}

fn target(kind: &str, id: i64) -> Result<Target> {
    Target::parse(kind, id).ok_or(Error::NotFound)
}

pub async fn assign_form(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((kind, id)): Path<(String, i64)>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let target = target(&kind, id)?;
    let obj = object(&state, user.lang, target).await?;
    let setting = pauses::load_all(&state.pool, now_ms())
        .await?
        .schedule_setting(target);
    Ok(Html(
        assign_page(
            &state,
            &user,
            &obj,
            setting.as_str().to_owned(),
            setting.schedule_id(),
            None,
        )
        .await?,
    ))
}

#[derive(Deserialize)]
pub struct AssignForm {
    csrf_token: String,
    #[serde(default)]
    choice: String,
    #[serde(default)]
    schedule: String,
}

pub async fn assign(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((kind, id)): Path<(String, i64)>,
    Form(form): Form<AssignForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let target = target(&kind, id)?;
    let obj = object(&state, user.lang, target).await?;
    let selected = form.schedule.trim().parse::<i64>().ok();
    let assignment = match (form.choice.as_str(), selected) {
        ("parent", _) => Some(Assignment::Parent),
        ("none", _) => Some(Assignment::None),
        ("schedule", Some(id)) => Some(Assignment::Schedule(id)),
        _ => None,
    };
    let result = match assignment {
        Some(a) => store::assign(&state.pool, target, a).await.map(|()| a),
        None => Err(Error::invalid(crate::i18n::Msg::new("schedule-required"))),
    };
    match result {
        Ok(a) => {
            tracing::info!(
                user = %user.username,
                kind,
                id,
                schedule = ?a.schedule_id(),
                choice = a.as_str(),
                "maintenance schedule assigned"
            );
            let changes = match a {
                Assignment::Parent => {
                    Changes::new().value_label("audit-field-schedule", "audit-inherited")
                }
                Assignment::None => {
                    Changes::new().value_label("audit-field-schedule", "audit-none")
                }
                Assignment::Schedule(schedule) => {
                    let name = store::get(&state.pool, schedule)
                        .await?
                        .map_or_else(|| schedule.to_string(), |s| s.name);
                    Changes::new().value("audit-field-schedule", name)
                }
            };
            let object = AuditObject::target(target, &obj.name);
            audit::record(&state, &user, Action::Changed, object, changes).await;
            pauses::sync_all(&state.pool, now_ms()).await?;
            Ok(Redirect::to(&href(&obj, Some("maintenance_saved"))).into_response())
        }
        Err(Error::Invalid(msg)) => {
            let error = Some(user.lang.msg(&msg));
            let html = assign_page(&state, &user, &obj, form.choice, selected, error).await?;
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
    use crate::storage::{devices::save_device, groups, sensors};
    use crate::web::testutil::*;

    #[tokio::test]
    async fn admins_manage_schedules_and_their_windows() {
        let h = harness(true).await;
        let (viewer, _) = login(&h, "bert").await;
        let (admin, csrf) = login(&h, "anna").await;
        assert_eq!(
            send(&h.app, get("/schedules", Some(&viewer))).await.status,
            StatusCode::FORBIDDEN
        );
        let page = send(&h.app, get("/schedules", Some(&admin))).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains("Noch keine Zeitpläne"), "{}", page.body);

        let reply = send(
            &h.app,
            post(
                "/schedules",
                Some(&admin),
                &format!("csrf_token={csrf}&name=Nachts"),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        let id: i64 = reply
            .location()
            .trim_start_matches("/schedules/")
            .split('?')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let again = send(
            &h.app,
            post(
                "/schedules",
                Some(&admin),
                &format!("csrf_token={csrf}&name=nachts"),
            ),
        )
        .await;
        assert_eq!(again.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(again.body.contains("existiert bereits"));

        let windows = format!("/schedules/{id}/windows");
        let weekly =
            format!("csrf_token={csrf}&kind=weekly&weekday=1&weekday=2&time=03%3A00&duration=15");
        let reply = send(&h.app, post(&windows, Some(&admin), &weekly)).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.body);
        let no_day = format!("csrf_token={csrf}&kind=weekly&time=03%3A00&duration=15");
        let reply = send(&h.app, post(&windows, Some(&admin), &no_day)).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(reply.body.contains("mindestens einen Wochentag"));
        let backwards =
            format!("csrf_token={csrf}&kind=once&start=2030-01-02T10%3A00&end=2030-01-02T09%3A00");
        let reply = send(&h.app, post(&windows, Some(&admin), &backwards)).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(reply.body.contains("nach dem Beginn"));
        assert!(reply.body.contains("value=\"2030-01-02T10:00\""), "kept");
        let once =
            format!("csrf_token={csrf}&kind=once&start=2030-01-02T10%3A00&end=2030-01-02T11%3A00");
        assert_eq!(
            send(&h.app, post(&windows, Some(&admin), &once))
                .await
                .status,
            StatusCode::SEE_OTHER
        );
        let wrong_csrf = weekly.replace(&csrf, "x");
        assert_eq!(
            send(&h.app, post(&windows, Some(&admin), &wrong_csrf))
                .await
                .status,
            StatusCode::FORBIDDEN
        );

        let page = send(&h.app, get(&format!("/schedules/{id}"), Some(&admin))).await;
        assert!(
            page.body.contains("Mo, Di ab 03:00 Uhr, 15 Min.<"),
            "{}",
            page.body
        );
        assert!(page.body.contains("2030-01-02 10:00 bis 2030-01-02 11:00"));
        let list = send(&h.app, get("/schedules", Some(&admin))).await;
        assert!(list.body.contains("Nachts"));
        assert!(list.body.contains("nächster Beginn"));

        let s = store::get(&h.state.pool, id).await.unwrap().unwrap();
        let window = s.windows[0].id;
        let reply = send(
            &h.app,
            post(
                &format!("/schedules/{id}/windows/{window}/delete"),
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        let reply = send(
            &h.app,
            post(
                &format!("/schedules/{id}"),
                Some(&admin),
                &format!("csrf_token={csrf}&name=Update"),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        let s = store::get(&h.state.pool, id).await.unwrap().unwrap();
        assert_eq!((s.name.as_str(), s.windows.len()), ("Update", 1));
        let reply = send(
            &h.app,
            post(
                &format!("/schedules/{id}/delete"),
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(reply.location(), "/schedules?msg=schedule_deleted");
        assert!(store::list(&h.state.pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_group_in_an_open_window_is_paused_and_logged() {
        let h = harness(true).await;
        let pool = &h.state.pool;
        let group = groups::create_group(pool, "Netzwerk", None).await.unwrap();
        let device = save_device(
            pool,
            None,
            DeviceInput {
                kind: Default::default(),
                name: "Switch".to_owned(),
                host: "192.0.2.1".to_owned(),
                group_id: Some(group),
                profile_id: None,
                polling: Polling::default(),
            },
        )
        .await
        .unwrap();
        let sensor = sensors::create_sensor(pool, device, "Ping", &SensorSpec::ping())
            .await
            .unwrap();
        let id = store::create(pool, "Nachts").await.unwrap();
        let now = now_ms();
        store::add_window(
            pool,
            id,
            Window::Once {
                start_ms: now - 60_000,
                end_ms: now + 3_600_000,
            },
        )
        .await
        .unwrap();
        let (viewer, vcsrf) = login(&h, "bert").await;
        let (admin, csrf) = login(&h, "anna").await;
        let path = format!("/maintenance/group/{group}");
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
                    &format!("csrf_token={vcsrf}&choice=none")
                )
            )
            .await
            .status,
            StatusCode::FORBIDDEN
        );
        let page = send(&h.app, get(&path, Some(&admin))).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(
            page.body.contains("Wartungsfenster für „Netzwerk“"),
            "{}",
            page.body
        );
        assert!(page.body.contains("Gilt jetzt: keins"));
        let missing = send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!("csrf_token={csrf}&choice=schedule"),
            ),
        )
        .await;
        assert_eq!(missing.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(missing.body.contains("Bitte einen Zeitplan wählen"));
        let reply = send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!("csrf_token={csrf}&choice=schedule&schedule={id}"),
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/groups/{group}?msg=maintenance_saved")
        );

        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&viewer))).await;
        assert!(
            page.body.contains("Wartungsfenster „Nachts“"),
            "{}",
            page.body
        );
        let page = send(
            &h.app,
            get(&format!("/maintenance/sensor/{sensor}"), Some(&admin)),
        )
        .await;
        assert!(
            page.body.contains("„Nachts“, von der Gruppe „Netzwerk“"),
            "{}",
            page.body
        );
        assert!(page.body.contains("offen bis"));
        let log = send(&h.app, get("/log", Some(&viewer))).await;
        assert!(
            log.body.contains("„Nachts“: 1 Sensor an 1 Gerät pausiert"),
            "{}",
            log.body
        );

        // "None" on the device ends it for the device and its sensor.
        let reply = send(
            &h.app,
            post(
                &format!("/maintenance/device/{device}"),
                Some(&admin),
                &format!("csrf_token={csrf}&choice=none"),
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/devices/{device}/sensors?msg=maintenance_saved")
        );
        let pauses = pauses::load_all(pool, now_ms()).await.unwrap();
        assert_eq!(pauses.sensor(sensor, device), None);
        assert!(
            pauses
                .group(group)
                .is_some_and(|w| w.schedule() == Some(id))
        );
        let log = send(&h.app, get("/log", Some(&viewer))).await;
        assert!(
            log.body.contains("„Nachts“: Sensoren laufen weiter"),
            "{}",
            log.body
        );
    }
}
