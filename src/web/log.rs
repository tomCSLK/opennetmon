//! Problem log: warnings, outages and their end over all sensors, and who paused or resumed what,
//! newest first. Only the newest [`status::EVENTS_KEPT`] status changes and
//! [`pauses::LOG_KEPT`] pause entries are stored, so the log reaches back that far.

use std::collections::HashMap;

use askama::Template;
use axum::extract::State;
use axum::response::Html;

use crate::devices::Device;
use crate::error::Result;
use crate::i18n::Lang;
use crate::pause::Target;
use crate::sensors::Unit;
use crate::sensors::format::format_value;
use crate::status::Status;
use crate::storage::pauses::{self, Action};
use crate::storage::{devices, groups, sensors, status};
use crate::web::auth::CurrentUser;
use crate::web::sensors::now_ms;
use crate::web::status_list::group_paths;
use crate::web::{AppState, Layout, time_html};

struct Entry {
    ts_ms: i64,
    at: String,
    status_class: &'static str,
    status_label: String,
    /// The status before, e.g. "vorher Warnung"; empty for a first poll.
    before: String,
    /// `None` for the pause of a whole device or group, see `scope`.
    sensor_id: Option<i64>,
    sensor_name: String,
    /// "ganzes Gerät", "ganze Gruppe".
    scope: String,
    device_id: Option<i64>,
    device_name: String,
    group: String,
    message: String,
    /// The planned end of a pause as `<time>` HTML.
    until: Option<String>,
    /// How long the problem lasted, lasts so far, or (for a recovery) lasted before.
    duration: String,
    ongoing: bool,
}

#[derive(Template)]
#[template(path = "log.html")]
struct LogTemplate {
    t: Lang,
    layout: Layout,
    entries: Vec<Entry>,
    kept: i64,
}

fn duration(t: Lang, ms: i64) -> String {
    format_value(t, Unit::Seconds, (ms.max(0) / 1_000) as f64)
}

pub async fn log(State(state): State<AppState>, user: CurrentUser) -> Result<Html<String>> {
    let t = user.lang;
    let paths = group_paths(&groups::list_groups(&state.pool).await?);
    let devices: HashMap<i64, Device> = devices::list_devices(&state.pool)
        .await?
        .into_iter()
        .map(|d| (d.id, d))
        .collect();
    let sensors: HashMap<i64, (String, i64)> = sensors::list_all_sensors(&state.pool)
        .await?
        .into_iter()
        .map(|s| (s.id, (s.name, s.device_id)))
        .collect();
    let now = now_ms();
    let mut entries = Vec::new();
    for e in status::problem_log(&state.pool).await? {
        let ev = e.event;
        let Some((sensor_name, device_id)) = sensors.get(&ev.sensor_id) else {
            continue;
        };
        let Some(device) = devices.get(device_id) else {
            continue;
        };
        let recovery = ev.to == Status::Up;
        let (duration, ongoing) = match (recovery, e.next_ts_ms) {
            (true, _) => (
                e.prev_ts_ms.map_or_else(String::new, |prev| {
                    t.get1("log-after", "duration", duration(t, ev.ts_ms - prev))
                }),
                false,
            ),
            (false, Some(next)) => (duration(t, next - ev.ts_ms), false),
            (false, None) => (duration(t, now - ev.ts_ms), true),
        };
        entries.push(Entry {
            ts_ms: ev.ts_ms,
            at: time_html(ev.ts_ms),
            status_class: ev.to.as_str(),
            status_label: t.get(ev.to.label_id()),
            before: ev.from.map_or_else(String::new, |from| {
                t.get1("log-before", "status", t.get(from.label_id()))
            }),
            sensor_id: Some(ev.sensor_id),
            sensor_name: sensor_name.clone(),
            scope: String::new(),
            device_id: Some(device.id),
            device_name: device.name.clone(),
            group: device
                .group_id
                .and_then(|g| paths.get(&g).cloned())
                .unwrap_or_default(),
            message: t.msg(&ev.message),
            until: None,
            duration,
            ongoing,
        });
    }
    for p in pauses::log_entries(&state.pool).await? {
        let (status_class, status_label, mut message) = match p.action {
            Action::Paused => (
                Status::Paused.as_str(),
                t.get(Status::Paused.label_id()),
                t.get1("log-paused-by", "user", p.user.clone().unwrap_or_default()),
            ),
            Action::Resumed => (
                Status::Up.as_str(),
                t.get("log-resumed"),
                t.get1("log-resumed-by", "user", p.user.clone().unwrap_or_default()),
            ),
            Action::Expired => (
                Status::Up.as_str(),
                t.get("log-expired"),
                t.get("log-expired-message"),
            ),
        };
        if !p.reason.is_empty() {
            message.push_str(" · ");
            message.push_str(&p.reason);
        }
        let (duration, ongoing) = match (p.action, p.end_ms) {
            (Action::Paused, Some(end)) => (duration(t, end - p.ts_ms), false),
            (Action::Paused, None) => (duration(t, now - p.ts_ms), true),
            _ => (String::new(), false),
        };
        let group_id = match p.target {
            Target::Group(id) => Some(id),
            _ => p
                .device
                .as_ref()
                .and_then(|(id, _)| devices.get(id))
                .and_then(|d| d.group_id),
        };
        entries.push(Entry {
            ts_ms: p.ts_ms,
            at: time_html(p.ts_ms),
            status_class,
            status_label,
            before: String::new(),
            sensor_id: match p.target {
                Target::Sensor(id) => Some(id),
                _ => None,
            },
            sensor_name: p.sensor.unwrap_or_default(),
            scope: match p.target {
                Target::Group(_) => t.get("log-scope-group"),
                Target::Device(_) => t.get("log-scope-device"),
                Target::Sensor(_) => String::new(),
            },
            device_id: p.device.as_ref().map(|(id, _)| *id),
            device_name: p.device.map(|(_, name)| name).unwrap_or_default(),
            group: group_id
                .and_then(|g| paths.get(&g).cloned())
                .or(p.group)
                .unwrap_or_default(),
            message,
            until: p
                .until_ms
                .filter(|_| p.action == Action::Paused)
                .map(time_html),
            duration,
            ongoing,
        });
    }
    // Both lists are newest first; merge them by time (stable, so ties keep their order).
    entries.sort_by_key(|e| std::cmp::Reverse(e.ts_ms));
    let page = LogTemplate {
        t,
        layout: user.layout(),
        entries,
        kept: status::EVENTS_KEPT,
    };
    Ok(Html(page.render()?))
}

#[cfg(test)]
mod tests {
    use crate::devices::{DeviceInput, Polling};
    use crate::sensors::spec::SensorSpec;
    use crate::status::Assessment;
    use crate::storage::sensors::create_sensor;
    use crate::storage::status::record_poll;
    use crate::web::testutil::*;

    #[tokio::test]
    async fn the_log_shows_problems_and_their_end_for_every_user() {
        let h = harness(true).await;
        let pool = &h.state.pool;
        let group = crate::storage::groups::create_group(pool, "Netzwerk", None)
            .await
            .unwrap();
        let device = crate::storage::devices::save_device(
            pool,
            None,
            DeviceInput {
                kind: Default::default(),
                name: "Switch <1>".to_owned(),
                host: "192.0.2.1".to_owned(),
                group_id: Some(group),
                profile_id: None,
                polling: Polling::default(),
            },
        )
        .await
        .unwrap();
        let sensor = create_sensor(pool, device, "Gateway", &SensorSpec::Ping)
            .await
            .unwrap();
        let ok = || Assessment::Healthy("OK".into());
        let fail = || Assessment::Failed("weg".into());
        for (ts, a) in [
            (1_000, ok()),
            (61_000, fail()),
            (121_000, fail()),
            (421_000, ok()),
        ] {
            record_poll(pool, sensor, ts, &[], &a).await.unwrap();
        }
        let (viewer, _) = login(&h, "bert").await;

        let page = send(&h.app, get("/log", Some(&viewer))).await;
        assert_eq!(page.status, axum::http::StatusCode::OK);
        let body = &page.body;
        // Newest first: the end of the outage, the outage, the warning; the first "up" is left out.
        let up = body.find("badge status-up").unwrap();
        let down = body.find("badge status-down").unwrap();
        let warning = body.find("badge status-warning").unwrap();
        assert!(up < down && down < warning, "{body}");
        assert_eq!(body.matches("<tr>").count(), 4, "header and three entries");
        assert!(
            body.contains("vorher Ausfall") && body.contains("nach 5 Min."),
            "{body}"
        );
        assert!(body.contains("Switch &#60;1&#62;") && body.contains("Netzwerk"));
        assert!(body.contains(&format!("href=\"/sensors/{sensor}\"")));
        assert!(body.contains("data-refresh-now"));

        let anonymous = send(&h.app, get("/log", None)).await;
        assert_eq!(anonymous.status, axum::http::StatusCode::SEE_OTHER);
    }
}
