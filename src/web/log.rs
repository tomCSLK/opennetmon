//! Problem log: warnings, outages and their end over all sensors, and who paused or resumed what,
//! newest first unless sorted by another column (`?sort=…&dir=…`, like the status lists); a filter field
//! (`static/table-filter.js`) searches the entries. Only the newest [`status::EVENTS_KEPT`] status changes and
//! [`pauses::LOG_KEPT`] pause entries are stored, so the log reaches back that far.

use std::collections::HashMap;

use askama::Template;
use axum::extract::{Query, State};
use axum::response::Html;
use serde::Deserialize;

use crate::devices::Device;
use crate::error::Result;
use crate::i18n::Lang;
use crate::ordering::natural_cmp;
use crate::pause::Target;
use crate::sensors::Unit;
use crate::sensors::format::format_value;
use crate::status::Status;
use crate::storage::pauses::{self, Action};
use crate::storage::{devices, groups, sensors, status};
use crate::web::auth::CurrentUser;
use crate::web::sensors::now_ms;
use crate::web::status_list::{Column, Dir, group_paths, missing_last, rank};
use crate::web::{AppState, Layout, time_html};

struct Entry {
    ts_ms: i64,
    at: String,
    /// For sorting; resumed and expired pauses count as up.
    status: Status,
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
    /// The same in milliseconds, for sorting.
    duration_ms: Option<i64>,
    ongoing: bool,
}

impl Entry {
    /// What the filter field searches, in lower case: status, sensor, device, group and message.
    fn filter_text(&self) -> String {
        [
            self.status_label.as_str(),
            &self.before,
            &self.sensor_name,
            &self.scope,
            &self.device_name,
            &self.group,
            &self.message,
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {
    When,
    Status,
    Sensor,
    Device,
    Duration,
}

impl SortKey {
    const ALL: [Self; 5] = [
        Self::When,
        Self::Status,
        Self::Sensor,
        Self::Device,
        Self::Duration,
    ];

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::When => "when",
            Self::Status => "status",
            Self::Sensor => "sensor",
            Self::Device => "device",
            Self::Duration => "duration",
        }
    }

    fn label_id(self) -> &'static str {
        match self {
            Self::When => "detail-when",
            Self::Status => "status",
            Self::Sensor => "sensor",
            Self::Device => "status-list-device",
            Self::Duration => "log-duration",
        }
    }

    /// Direction of the first click on a column: newest and longest first.
    fn first_dir(self) -> Dir {
        match self {
            Self::When | Self::Duration => Dir::Desc,
            Self::Status | Self::Sensor | Self::Device => Dir::Asc,
        }
    }
}

/// Sorts by `key`; ties newest first (the input order).
fn sort_entries(entries: &mut [Entry], key: SortKey, dir: Dir) {
    entries.sort_by(|a, b| {
        let primary = match key {
            SortKey::When => dir.apply(a.ts_ms.cmp(&b.ts_ms)),
            SortKey::Status => dir.apply(rank(a.status).cmp(&rank(b.status))),
            SortKey::Sensor => dir.apply(
                natural_cmp(&a.sensor_name, &b.sensor_name).then_with(|| a.scope.cmp(&b.scope)),
            ),
            SortKey::Device => dir.apply(natural_cmp(&a.device_name, &b.device_name)),
            SortKey::Duration => {
                missing_last(a.duration_ms, b.duration_ms, |x, y| dir.apply(x.cmp(y)))
            }
        };
        primary.then_with(|| b.ts_ms.cmp(&a.ts_ms))
    });
}

#[derive(Deserialize)]
pub struct LogQuery {
    sort: Option<String>,
    dir: Option<String>,
}

#[derive(Template)]
#[template(path = "log.html")]
struct LogTemplate {
    t: Lang,
    layout: Layout,
    columns: Vec<Column>,
    entries: Vec<Entry>,
    kept: i64,
}

fn duration(t: Lang, ms: i64) -> String {
    format_value(t, Unit::Seconds, (ms.max(0) / 1_000) as f64)
}

pub async fn log(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<LogQuery>,
) -> Result<Html<String>> {
    let t = user.lang;
    let key = query
        .sort
        .as_deref()
        .and_then(SortKey::parse)
        .unwrap_or(SortKey::When);
    let dir = query
        .dir
        .as_deref()
        .and_then(Dir::parse)
        .unwrap_or_else(|| key.first_dir());
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
        let (duration_ms, ongoing) = match (recovery, e.next_ts_ms) {
            (true, _) => (e.prev_ts_ms.map(|prev| ev.ts_ms - prev), false),
            (false, Some(next)) => (Some(next - ev.ts_ms), false),
            (false, None) => (Some(now - ev.ts_ms), true),
        };
        let duration = match (recovery, duration_ms) {
            (_, None) => String::new(),
            (true, Some(ms)) => t.get1("log-after", "duration", duration(t, ms)),
            (false, Some(ms)) => duration(t, ms),
        };
        entries.push(Entry {
            ts_ms: ev.ts_ms,
            at: time_html(ev.ts_ms),
            status: ev.to,
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
            duration_ms,
            ongoing,
        });
    }
    for p in pauses::log_entries(&state.pool).await? {
        let status = match p.action {
            Action::Paused => Status::Paused,
            Action::Resumed | Action::Expired => Status::Up,
        };
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
        let (duration_ms, ongoing) = match (p.action, p.end_ms) {
            (Action::Paused, Some(end)) => (Some(end - p.ts_ms), false),
            (Action::Paused, None) => (Some(now - p.ts_ms), true),
            _ => (None, false),
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
            status,
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
            duration: duration_ms.map_or_else(String::new, |ms| duration(t, ms)),
            duration_ms,
            ongoing,
        });
    }
    // Both lists are newest first; merge them by time (stable, so ties keep their order), then sort.
    entries.sort_by_key(|e| std::cmp::Reverse(e.ts_ms));
    sort_entries(&mut entries, key, dir);
    let mut columns: Vec<Column> = SortKey::ALL
        .into_iter()
        .map(|k| {
            let next = if k == key { dir.flip() } else { k.first_dir() };
            Column {
                label: t.get(k.label_id()),
                href: Some(format!("/log?sort={}&dir={}", k.as_str(), next.as_str())),
                aria_sort: if k == key { dir.aria_sort() } else { "" },
                right: false,
            }
        })
        .collect();
    // When, status, sensor, device, message, duration.
    columns.insert(
        4,
        Column {
            label: t.get("message"),
            href: None,
            aria_sort: "",
            right: false,
        },
    );
    let page = LogTemplate {
        t,
        layout: user.layout(),
        columns,
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
        assert_eq!(body.matches("<tr data-filter=").count(), 3, "three entries");
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

    #[tokio::test]
    async fn the_log_sorts_by_its_columns_and_offers_a_search() {
        let h = harness(true).await;
        let pool = &h.state.pool;
        let device = crate::storage::devices::save_device(
            pool,
            None,
            DeviceInput {
                kind: Default::default(),
                name: "Router".to_owned(),
                host: "192.0.2.1".to_owned(),
                group_id: None,
                profile_id: None,
                polling: Polling::default(),
            },
        )
        .await
        .unwrap();
        let alpha = create_sensor(pool, device, "Alpha", &SensorSpec::Ping)
            .await
            .unwrap();
        let beta = create_sensor(pool, device, "Beta", &SensorSpec::Ping)
            .await
            .unwrap();
        let ok = || Assessment::Healthy("OK".into());
        let fail = || Assessment::Failed("weg".into());
        // Alpha: a warning of 10 minutes long ago; Beta: a warning of 1 minute later.
        for (sensor, ts, a) in [
            (alpha, 1_000, ok()),
            (alpha, 60_000, fail()),
            (alpha, 660_000, ok()),
            (beta, 1_000, ok()),
            (beta, 700_000, fail()),
            (beta, 760_000, ok()),
        ] {
            record_poll(pool, sensor, ts, &[], &a).await.unwrap();
        }
        let (viewer, _) = login(&h, "bert").await;
        let order = |body: &str| -> Vec<(usize, &'static str)> {
            let mut found = Vec::new();
            for name in ["Alpha", "Beta"] {
                let needle = format!("<strong>{name}</strong>");
                found.extend(body.match_indices(&needle).map(|(i, _)| (i, name)));
            }
            found.sort();
            found
        };
        let names = |body: &str| order(body).into_iter().map(|(_, n)| n).collect::<Vec<_>>();

        // Default: newest first, the time column sorted descending.
        let page = send(&h.app, get("/log", Some(&viewer))).await;
        assert_eq!(names(&page.body), ["Beta", "Beta", "Alpha", "Alpha"]);
        assert!(
            page.body.contains(
                "aria-sort=\"descending\"><a class=\"sort\" href=\"/log?sort=when&#38;dir=asc\">"
            ),
            "{}",
            page.body
        );
        assert!(page.body.contains("data-table-filter=\"log-entries\""));
        assert!(
            page.body
                .contains("data-filter=\"in ordnung vorher warnung beta router ok\"")
        );

        let by_sensor = send(&h.app, get("/log?sort=sensor&dir=asc", Some(&viewer))).await;
        assert_eq!(names(&by_sensor.body), ["Alpha", "Alpha", "Beta", "Beta"]);
        let by_duration = send(&h.app, get("/log?sort=duration&dir=desc", Some(&viewer))).await;
        assert_eq!(names(&by_duration.body), ["Alpha", "Alpha", "Beta", "Beta"]);
        let shortest = send(&h.app, get("/log?sort=duration&dir=asc", Some(&viewer))).await;
        assert_eq!(names(&shortest.body), ["Beta", "Beta", "Alpha", "Alpha"]);
        // Unknown values fall back to the default.
        let odd = send(&h.app, get("/log?sort=nonsense&dir=up", Some(&viewer))).await;
        assert_eq!(names(&odd.body), ["Beta", "Beta", "Alpha", "Alpha"]);
    }
}
