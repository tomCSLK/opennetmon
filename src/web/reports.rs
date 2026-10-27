//! Reports like PRTG's (for every user): availability and downtime per device or sensor over the
//! last 7 or 30 days or a calendar month (in the time zone of the server), sortable, as a page
//! (`/reports`) and as CSV (`/reports.csv`). See [`crate::availability`].

use std::cmp::Ordering;
use std::collections::HashMap;

use askama::Template;
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::{Html, IntoResponse, Response};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use serde::Deserialize;

use crate::availability::{self, Basis, DAY_MS, Part, Totals};
use crate::error::Result;
use crate::i18n::Lang;
use crate::ordering::natural_cmp;
use crate::sensors::Unit;
use crate::sensors::format::format_value;
use crate::sensors::spec::SensorSpec;
use crate::storage::{devices, groups, pauses, sensors};
use crate::web::auth::CurrentUser;
use crate::web::sensors::now_ms;
use crate::web::status_list::{Column, Dir, group_paths, missing_last};
use crate::web::{AppState, Layout};

/// How many past months the month choice offers.
const MONTHS_OFFERED: i32 = 12;
const MONTH_IDS: [&str; 12] = [
    "month-1", "month-2", "month-3", "month-4", "month-5", "month-6", "month-7", "month-8",
    "month-9", "month-10", "month-11", "month-12",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Period {
    Days7,
    Days30,
    /// A calendar month: year and month.
    Month(i16, i8),
}

impl Period {
    fn key(self) -> String {
        match self {
            Self::Days7 => "7d".to_owned(),
            Self::Days30 => "30d".to_owned(),
            Self::Month(y, m) => format!("{y:04}-{m:02}"),
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "7d" => Some(Self::Days7),
            "30d" => Some(Self::Days30),
            _ => {
                let (y, m) = text.split_once('-')?;
                let (y, m) = (y.parse().ok()?, m.parse().ok()?);
                Date::new(y, m, 1).ok().map(|_| Self::Month(y, m))
            }
        }
    }

    /// `from..to` in UTC milliseconds; a month in the zone `tz`, the current one up to now.
    fn range(self, now_ms: i64, tz: &TimeZone) -> Option<(i64, i64)> {
        match self {
            Self::Days7 => Some((now_ms - 7 * DAY_MS, now_ms)),
            Self::Days30 => Some((now_ms - 30 * DAY_MS, now_ms)),
            Self::Month(y, m) => {
                let first = Date::new(y, m, 1).ok()?;
                let start = |d: Date| -> Option<i64> {
                    Some(d.to_zoned(tz.clone()).ok()?.timestamp().as_millisecond())
                };
                let from = start(first)?;
                let to = start(first.checked_add(1.month()).ok()?)?;
                (from < now_ms).then_some((from, to.min(now_ms)))
            }
        }
    }
}

/// The current month and the ones before it, newest first.
fn months(now_ms: i64, tz: &TimeZone) -> Vec<(i16, i8)> {
    let Ok(now) = Timestamp::from_millisecond(now_ms) else {
        return Vec::new();
    };
    let first = now.to_zoned(tz.clone()).date().first_of_month();
    (0..MONTHS_OFFERED)
        .filter_map(|back| first.checked_sub(back.months()).ok())
        .map(|d| (d.year(), d.month()))
        .collect()
}

fn month_label(t: Lang, (y, m): (i16, i8)) -> String {
    let name = usize::try_from(m - 1)
        .ok()
        .and_then(|i| MONTH_IDS.get(i))
        .map_or_else(String::new, |id| t.get(id));
    format!("{name} {y}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Devices,
    Sensors,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Name,
    Availability,
    Downtime,
}

impl SortKey {
    fn parse(s: &str) -> Option<Self> {
        [Self::Name, Self::Availability, Self::Downtime]
            .into_iter()
            .find(|k| k.as_str() == s)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Availability => "availability",
            Self::Downtime => "downtime",
        }
    }

    /// The worst first: lowest availability, longest downtime.
    fn first_dir(self) -> Dir {
        match self {
            Self::Name | Self::Availability => Dir::Asc,
            Self::Downtime => Dir::Desc,
        }
    }
}

#[derive(Deserialize, Default)]
pub struct ReportQuery {
    period: Option<String>,
    view: Option<String>,
    sort: Option<String>,
    dir: Option<String>,
}

struct Settings {
    period: Period,
    view: View,
    key: SortKey,
    dir: Dir,
}

impl Settings {
    fn from(q: &ReportQuery) -> Self {
        let key = q
            .sort
            .as_deref()
            .and_then(SortKey::parse)
            .unwrap_or(SortKey::Availability);
        Self {
            period: q
                .period
                .as_deref()
                .and_then(Period::parse)
                .unwrap_or(Period::Days30),
            view: match q.view.as_deref() {
                Some("sensors") => View::Sensors,
                _ => View::Devices,
            },
            key,
            dir: q
                .dir
                .as_deref()
                .and_then(Dir::parse)
                .unwrap_or(key.first_dir()),
        }
    }

    fn query(&self, key: SortKey, dir: Dir) -> String {
        format!(
            "period={}&view={}&sort={}&dir={}",
            self.period.key(),
            match self.view {
                View::Devices => "devices",
                View::Sensors => "sensors",
            },
            key.as_str(),
            dir.as_str()
        )
    }
}

/// One line of the report.
struct Line {
    href: String,
    name: String,
    /// The device of a sensor, the group path of a device.
    context: String,
    percent: Option<f64>,
    down_ms: Option<i64>,
    counted_ms: i64,
    /// What a device's value is based on.
    basis: String,
}

/// `99,95 %`, rounded down so that a short outage never shows as 100 %.
pub fn percent_text(t: Lang, value: f64) -> String {
    let floored = (value * 100.0).floor() / 100.0;
    format!("{floored:.2} %").replace('.', &t.decimal_separator().to_string())
}

fn duration_text(t: Lang, ms: i64) -> String {
    format_value(t, Unit::Seconds, (ms.max(0) / 1_000) as f64)
}

async fn lines(state: &AppState, t: Lang, s: &Settings, from: i64, to: i64) -> Result<Vec<Line>> {
    let totals = crate::storage::availability::totals(&state.pool, from, to, None).await?;
    let all_groups = groups::list_groups(&state.pool).await?;
    let paths = group_paths(&all_groups);
    let device_list = devices::list_devices(&state.pool).await?;
    let sensor_list = sensors::list_all_sensors(&state.pool).await?;
    let names: HashMap<i64, &str> = device_list
        .iter()
        .map(|d| (d.id, d.name.as_str()))
        .collect();
    let mut lines = Vec::new();
    match s.view {
        View::Sensors => {
            for r in &sensor_list {
                let tot = totals.get(&r.id).copied().unwrap_or_default();
                lines.push(Line {
                    href: format!("/sensors/{}", r.id),
                    name: r.name.clone(),
                    context: names
                        .get(&r.device_id)
                        .copied()
                        .unwrap_or_default()
                        .to_owned(),
                    percent: tot.percent(),
                    down_ms: (tot.counted_ms() > 0).then_some(tot.down_ms),
                    counted_ms: tot.counted_ms(),
                    basis: String::new(),
                });
            }
        }
        View::Devices => {
            let pauses = pauses::load_all(&state.pool, now_ms()).await?;
            for d in &device_list {
                let master = pauses.master_of(d.id);
                let parts: Vec<Part> = sensor_list
                    .iter()
                    .filter(|r| r.device_id == d.id)
                    .map(|r| Part {
                        sensor_id: r.id,
                        master: master == Some(r.id),
                        ping: matches!(r.spec, SensorSpec::Ping(_)),
                        totals: totals.get(&r.id).copied().unwrap_or_default(),
                    })
                    .collect();
                let (percent, down_ms, basis) = availability::device(&parts);
                let sensor_name = |id: i64| {
                    sensor_list
                        .iter()
                        .find(|r| r.id == id)
                        .map_or("", |r| r.name.as_str())
                };
                let counted_ms = match basis {
                    Basis::Master(id) | Basis::Ping(id) => {
                        totals.get(&id).map_or(0, Totals::counted_ms)
                    }
                    Basis::Mean => parts
                        .iter()
                        .map(|p| p.totals.counted_ms())
                        .max()
                        .unwrap_or(0),
                };
                lines.push(Line {
                    href: format!("/devices/{}/sensors", d.id),
                    name: d.name.clone(),
                    context: d
                        .group_id
                        .and_then(|g| paths.get(&g).cloned())
                        .unwrap_or_default(),
                    percent,
                    down_ms,
                    counted_ms,
                    basis: match basis {
                        Basis::Master(id) => {
                            t.get1("report-basis-master", "sensor", sensor_name(id))
                        }
                        Basis::Ping(id) => t.get1("report-basis-ping", "sensor", sensor_name(id)),
                        Basis::Mean => t.get1("report-basis-mean", "count", parts.len()),
                    },
                });
            }
        }
    }
    sort(&mut lines, s.key, s.dir);
    Ok(lines)
}

fn sort(lines: &mut [Line], key: SortKey, dir: Dir) {
    let by_name = |a: &Line, b: &Line| {
        natural_cmp(&a.name, &b.name).then_with(|| natural_cmp(&a.context, &b.context))
    };
    lines.sort_by(|a, b| {
        let ord = match key {
            SortKey::Name => dir.apply(by_name(a, b)),
            // Lines without data stay last in both directions.
            SortKey::Availability => missing_last(a.percent, b.percent, |x, y| {
                dir.apply(x.partial_cmp(y).unwrap_or(Ordering::Equal))
            }),
            SortKey::Downtime => missing_last(a.down_ms, b.down_ms, |x, y| dir.apply(x.cmp(y))),
        };
        ord.then_with(|| by_name(a, b))
    });
}

struct Row {
    href: String,
    name: String,
    context: String,
    availability: String,
    /// Badge class for the color of the value; empty without data.
    level: &'static str,
    downtime: String,
    counted: String,
    basis: String,
}

#[derive(Template)]
#[template(path = "reports.html")]
struct ReportTemplate {
    t: Lang,
    layout: Layout,
    /// (key, label, selected)
    periods: Vec<(String, String, bool)>,
    view_devices: bool,
    period_text: String,
    columns: Vec<Column>,
    rows: Vec<Row>,
    csv_href: String,
    devices_href: String,
    sensors_href: String,
}

/// The period from the query, its range, or the default when it lies in the future.
fn resolve(s: &mut Settings, now: i64, tz: &TimeZone) -> (i64, i64) {
    match s.period.range(now, tz) {
        Some(range) => range,
        None => {
            s.period = Period::Days30;
            (now - 30 * DAY_MS, now)
        }
    }
}

pub async fn page(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<ReportQuery>,
) -> Result<Html<String>> {
    let t = user.lang;
    let tz = crate::schedule::time_zone();
    let now = now_ms();
    let mut s = Settings::from(&query);
    let (from, to) = resolve(&mut s, now, tz);
    let lines = lines(&state, t, &s, from, to).await?;
    let mut periods = vec![
        (
            "7d".to_owned(),
            t.get("report-period-7d"),
            s.period == Period::Days7,
        ),
        (
            "30d".to_owned(),
            t.get("report-period-30d"),
            s.period == Period::Days30,
        ),
    ];
    for (y, m) in months(now, tz) {
        let p = Period::Month(y, m);
        periods.push((p.key(), month_label(t, (y, m)), s.period == p));
    }
    let period_text = periods
        .iter()
        .find(|(_, _, sel)| *sel)
        .map(|(_, label, _)| label.clone())
        .unwrap_or_default();
    let name_label = match s.view {
        View::Devices => t.get("status-list-device"),
        View::Sensors => t.get("sensor"),
    };
    let columns = [
        (SortKey::Name, name_label, false),
        (SortKey::Availability, t.get("report-availability"), true),
        (SortKey::Downtime, t.get("report-downtime"), true),
    ]
    .into_iter()
    .map(|(k, label, right)| {
        let next = if k == s.key {
            s.dir.flip()
        } else {
            k.first_dir()
        };
        Column {
            label,
            href: Some(format!("/reports?{}", s.query(k, next))),
            aria_sort: if k == s.key { s.dir.aria_sort() } else { "" },
            right,
        }
    })
    .collect();
    let rows = lines
        .into_iter()
        .map(|l| Row {
            availability: l
                .percent
                .map_or_else(|| t.get("report-no-data"), |p| percent_text(t, p)),
            level: match l.percent {
                Some(p) if p >= 99.0 => "status-up",
                Some(p) if p >= 95.0 => "status-warning",
                Some(_) => "status-down",
                None => "",
            },
            downtime: l
                .down_ms
                .map_or_else(String::new, |ms| duration_text(t, ms)),
            counted: if l.counted_ms > 0 {
                t.get1(
                    "report-counted",
                    "time",
                    duration_text(t, l.counted_ms).as_str(),
                )
            } else {
                String::new()
            },
            href: l.href,
            name: l.name,
            context: l.context,
            basis: l.basis,
        })
        .collect();
    let view_query = |view: &str| {
        format!(
            "/reports?period={}&view={view}&sort={}&dir={}",
            s.period.key(),
            s.key.as_str(),
            s.dir.as_str()
        )
    };
    let page = ReportTemplate {
        layout: user.layout(),
        periods,
        view_devices: s.view == View::Devices,
        period_text,
        columns,
        rows,
        csv_href: format!("/reports.csv?{}", s.query(s.key, s.dir)),
        devices_href: view_query("devices"),
        sensors_href: view_query("sensors"),
        t,
    };
    Ok(Html(page.render()?))
}

/// One CSV field: quoted, and text a spreadsheet would run as a formula starts with `'`.
fn csv_field(text: &str) -> String {
    let text = if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{text}")
    } else {
        text.to_owned()
    };
    format!("\"{}\"", text.replace('"', "\"\""))
}

pub async fn csv(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<ReportQuery>,
) -> Result<Response> {
    let tz = crate::schedule::time_zone();
    let now = now_ms();
    let mut s = Settings::from(&query);
    let (from, to) = resolve(&mut s, now, tz);
    let lines = lines(&state, user.lang, &s, from, to).await?;
    let (name, context) = match s.view {
        View::Devices => ("device", "group"),
        View::Sensors => ("sensor", "device"),
    };
    let mut out = format!(
        "{name},{context},availability_percent,downtime_seconds,counted_seconds,basis,from,to\r\n"
    );
    let iso = |ms: i64| {
        Timestamp::from_millisecond(ms)
            .map(|ts| ts.to_string())
            .unwrap_or_default()
    };
    for l in lines {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{}\r\n",
            csv_field(&l.name),
            csv_field(&l.context),
            l.percent.map_or_else(String::new, |p| format!("{p:.4}")),
            l.down_ms
                .map_or_else(String::new, |ms| (ms / 1_000).to_string()),
            l.counted_ms / 1_000,
            csv_field(&l.basis),
            iso(from),
            iso(to),
        ));
    }
    let file = format!(
        "attachment; filename=\"opennetmon-availability-{name}s-{}.csv\"",
        s.period.key()
    );
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_owned()),
            (header::CONTENT_DISPOSITION, file),
        ],
        out,
    )
        .into_response())
}

/// Availability of one sensor for its detail page: (label, value, downtime) for 24 h, 7 and 30 days.
pub async fn sensor_summary(
    state: &AppState,
    t: Lang,
    sensor_id: i64,
) -> Result<Vec<(String, String, String)>> {
    let now = now_ms();
    let mut out = Vec::new();
    for (label, days) in [
        ("report-period-24h", 1),
        ("report-period-7d", 7),
        ("report-period-30d", 30),
    ] {
        let totals = crate::storage::availability::totals(
            &state.pool,
            now - days * DAY_MS,
            now,
            Some(sensor_id),
        )
        .await?;
        let tot = totals.get(&sensor_id).copied().unwrap_or_default();
        let (value, down) = match tot.percent() {
            Some(p) => (percent_text(t, p), duration_text(t, tot.down_ms)),
            None => (t.get("report-no-data"), String::new()),
        };
        out.push((t.get(label), value, down));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_round_down() {
        assert_eq!(percent_text(Lang::De, 99.999), "99,99 %");
        assert_eq!(percent_text(Lang::En, 100.0), "100.00 %");
        assert_eq!(percent_text(Lang::De, 50.0), "50,00 %");
    }

    #[test]
    fn months_are_ranges_in_the_zone_of_the_server() {
        let tz = TimeZone::get("Europe/Berlin").unwrap();
        let at = |s: &str| crate::schedule::parse_local(s, &tz).unwrap();
        let now = at("2026-10-27T12:00");
        assert_eq!(
            Period::Month(2026, 9).range(now, &tz),
            Some((at("2026-09-01T00:00"), at("2026-10-01T00:00")))
        );
        assert_eq!(
            Period::Month(2026, 10).range(now, &tz),
            Some((at("2026-10-01T00:00"), now)),
            "the current month up to now"
        );
        assert_eq!(Period::Month(2026, 11).range(now, &tz), None);
        let offered = months(now, &tz);
        assert_eq!(offered.len(), 12);
        assert_eq!((offered[0], offered[11]), ((2026, 10), (2025, 11)));
        assert_eq!(Period::parse("2026-13"), None);
        assert_eq!(Period::parse("2026-02"), Some(Period::Month(2026, 2)));
        assert_eq!(Period::parse("x"), None);
    }

    #[test]
    fn csv_fields_are_quoted_and_never_formulas() {
        assert_eq!(csv_field("Switch \"Büro\""), "\"Switch \"\"Büro\"\"\"");
        assert_eq!(csv_field("=HYPERLINK(1)"), "\"'=HYPERLINK(1)\"");
        assert_eq!(csv_field("-5"), "\"'-5\"");
    }
}

#[cfg(test)]
mod handler_tests {
    use axum::http::StatusCode;

    use crate::devices::{DeviceInput, Polling};
    use crate::sensors::spec::SensorSpec;
    use crate::status::Status;
    use crate::storage::{devices::save_device, groups, sensors::create_sensor};
    use crate::web::sensors::now_ms;
    use crate::web::testutil::*;

    #[tokio::test]
    async fn every_user_sees_availability_per_device_and_sensor_and_as_csv() {
        let h = harness(true).await;
        let pool = &h.state.pool;
        let group = groups::create_group(pool, "Netzwerk", None).await.unwrap();
        let device = save_device(
            pool,
            None,
            DeviceInput {
                kind: Default::default(),
                name: "=Router".to_owned(),
                host: "192.0.2.1".to_owned(),
                group_id: Some(group),
                profile_id: None,
                polling: Polling::default(),
            },
        )
        .await
        .unwrap();
        let ping = create_sensor(pool, device, "Ping", &SensorSpec::ping())
            .await
            .unwrap();
        create_sensor(pool, device, "Uptime", &SensorSpec::Uptime)
            .await
            .unwrap();
        let now = now_ms();
        let mut conn = pool.acquire().await.unwrap();
        let minute = 60_000;
        for i in 0..9 {
            let at = now - 3_600_000 + i * minute;
            crate::storage::availability::count(&mut conn, ping, Status::Up, at, at + minute)
                .await
                .unwrap();
        }
        let at = now - 3_600_000 + 9 * minute;
        crate::storage::availability::count(&mut conn, ping, Status::Down, at, at + minute)
            .await
            .unwrap();
        drop(conn);

        let (viewer, _) = login(&h, "bert").await;
        let page = send(&h.app, get("/reports", Some(&viewer))).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains("=Router"), "{}", page.body);
        assert!(page.body.contains("90,00 %"));
        assert!(page.body.contains("nach „Ping“"));
        assert!(page.body.contains("Netzwerk"));
        let sensors = send(
            &h.app,
            get("/reports?view=sensors&period=7d", Some(&viewer)),
        )
        .await;
        assert!(sensors.body.contains("Uptime"));
        assert!(sensors.body.contains("keine Daten"));
        assert!(sensors.body.contains("aria-current=\"page\">Sensoren"));
        // A month in the future falls back to 30 days.
        let future = send(&h.app, get("/reports?period=2999-01", Some(&viewer))).await;
        assert_eq!(future.status, StatusCode::OK);
        assert!(future.body.contains("30 Tage"));

        let csv = send(
            &h.app,
            get("/reports.csv?view=devices&period=7d", Some(&viewer)),
        )
        .await;
        assert_eq!(csv.status, StatusCode::OK);
        assert!(
            csv.headers[axum::http::header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/csv")
        );
        let mut lines = csv.body.lines();
        assert!(
            lines
                .next()
                .unwrap()
                .starts_with("device,group,availability_percent")
        );
        let line = lines.next().unwrap();
        assert!(
            line.starts_with("\"'=Router\",\"Netzwerk\",90.0000,60,600,"),
            "{line}"
        );

        let detail = send(&h.app, get(&format!("/sensors/{ping}"), Some(&viewer))).await;
        assert!(
            detail.body.contains("Verfügbarkeit 24 Std."),
            "{}",
            detail.body
        );
        assert!(detail.body.contains("Ausfall 1 Min. 0 s"));
    }
}
