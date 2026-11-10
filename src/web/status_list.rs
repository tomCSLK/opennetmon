//! Sensor lists by status, opened from the tiles on the start page: all sensors, those that are
//! up, in warning, down, or inactive/unknown. Sortable by status, sensor, device, current value
//! and the time of the last status change. `?group=<id>` limits a list to a group and its
//! subgroups (the tiles of a group page), `?q=<text>` to the sensors whose name, device, address,
//! group or message contain the text (the search in the header).

use std::cmp::Ordering;
use std::collections::HashMap;

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::response::Html;
use serde::Deserialize;

use crate::devices::{Device, Group};
use crate::error::{Error, Result};
use crate::i18n::Lang;
use crate::ordering::natural_cmp;
use crate::sensors::Unit;
use crate::sensors::format::format_value;
use crate::status::Status;
use crate::storage::{devices, groups, sensors, status};
use crate::web::auth::CurrentUser;
use crate::web::sensors::{fresh_values, now_ms};
use crate::web::{AppState, HeaderCounts, Layout, time_html, url_param};

/// Longest search text; anything after it is ignored.
const MAX_SEARCH_CHARS: usize = 100;

/// The counters of the header: sensors up, in warning and down, counted like the tiles of the overview.
pub async fn header_counts(state: &AppState) -> Result<HeaderCounts> {
    let states = status::list_states(&state.pool).await?;
    let pauses = crate::storage::pauses::load_all(&state.pool, now_ms()).await?;
    let mut counts = HeaderCounts::default();
    for sensor in sensors::list_all_sensors(&state.pool).await? {
        let shown = crate::status::shown_status(
            pauses.sensor(sensor.id, sensor.device_id).is_some(),
            states.get(&sensor.id).map(|s| s.status),
        );
        match shown {
            Status::Up => counts.up += 1,
            Status::Warning => counts.warning += 1,
            Status::Down => counts.down += 1,
            Status::Unknown | Status::Paused => {}
        }
    }
    Ok(counts)
}

/// Which sensors a list shows; the same split as the tiles on the start page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Up,
    Warning,
    Down,
    /// Paused and unknown, like the last tile.
    Inactive,
}

impl Filter {
    const ALL: [Self; 5] = [
        Self::All,
        Self::Up,
        Self::Warning,
        Self::Down,
        Self::Inactive,
    ];

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.as_str() == s)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Up => "up",
            Self::Warning => "warning",
            Self::Down => "down",
            Self::Inactive => "inactive",
        }
    }

    fn label_id(self) -> &'static str {
        match self {
            Self::All => "status-list-all",
            Self::Up => "status-up",
            Self::Warning => "status-warning",
            Self::Down => "status-down",
            Self::Inactive => "overview-paused-unknown",
        }
    }

    fn matches(self, status: Status) -> bool {
        match self {
            Self::All => true,
            Self::Up => status == Status::Up,
            Self::Warning => status == Status::Warning,
            Self::Down => status == Status::Down,
            Self::Inactive => matches!(status, Status::Paused | Status::Unknown),
        }
    }

    /// Problems newest first; healthy and inactive sensors by device.
    fn default_sort(self) -> (SortKey, Dir) {
        match self {
            Self::All => (SortKey::Status, Dir::Asc),
            Self::Warning | Self::Down => (SortKey::Since, Dir::Desc),
            Self::Up | Self::Inactive => (SortKey::Device, Dir::Asc),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Status,
    Sensor,
    Device,
    Value,
    Since,
}

impl SortKey {
    const ALL: [Self; 5] = [
        Self::Status,
        Self::Sensor,
        Self::Device,
        Self::Value,
        Self::Since,
    ];

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Sensor => "sensor",
            Self::Device => "device",
            Self::Value => "value",
            Self::Since => "since",
        }
    }

    fn label_id(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Sensor => "sensor",
            Self::Device => "status-list-device",
            Self::Value => "sensors-current",
            Self::Since => "status-list-since",
        }
    }

    /// Direction of the first click on a column: the largest values and newest changes first.
    fn first_dir(self) -> Dir {
        match self {
            Self::Value | Self::Since => Dir::Desc,
            Self::Status | Self::Sensor | Self::Device => Dir::Asc,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dir {
    Asc,
    Desc,
}

impl Dir {
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "asc" => Some(Self::Asc),
            "desc" => Some(Self::Desc),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }

    pub(crate) fn flip(self) -> Self {
        match self {
            Self::Asc => Self::Desc,
            Self::Desc => Self::Asc,
        }
    }

    /// `aria-sort` of the column sorted this way.
    pub(crate) fn aria_sort(self) -> &'static str {
        match self {
            Self::Asc => "ascending",
            Self::Desc => "descending",
        }
    }

    pub(crate) fn apply(self, ord: Ordering) -> Ordering {
        match self {
            Self::Asc => ord,
            Self::Desc => ord.reverse(),
        }
    }
}

/// Worst first, inactive last.
pub(crate) fn rank(status: Status) -> u8 {
    match status {
        Status::Down => 0,
        Status::Warning => 1,
        Status::Unknown => 2,
        Status::Up => 3,
        Status::Paused => 4,
    }
}

struct Row {
    id: i64,
    name: String,
    device_id: i64,
    device_name: String,
    /// Groups above the device, top level first (empty without a group).
    group: String,
    status: Status,
    status_label: String,
    message: String,
    /// Why the sensor is paused, instead of the message.
    pause: Option<crate::web::pause::PauseNote>,
    /// When the sensor entered its status (not for inactive or never polled sensors).
    since_ms: Option<i64>,
    since: String,
    duration: String,
    values: Vec<String>,
    value_title: String,
    /// Rank of the unit and sum of the current values, for sorting.
    total: Option<(u8, f64)>,
}

/// Sensors with the same unit stay together when sorted by value, traffic first; only values
/// within a unit are compared.
fn unit_rank(unit: Unit) -> u8 {
    match unit {
        Unit::BitsPerSecond => 0,
        Unit::PerSecond => 1,
        Unit::Milliseconds => 2,
        Unit::Percent => 3,
        Unit::Count => 4,
        Unit::Seconds => 5,
        Unit::State => 6,
        Unit::Bytes => 7,
        Unit::Celsius => 8,
        Unit::Days => 9,
    }
}

/// Compares two optional keys with `cmp`; rows without a key always come last.
pub(crate) fn missing_last<T>(
    a: Option<T>,
    b: Option<T>,
    cmp: impl Fn(&T, &T) -> Ordering,
) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => cmp(&a, &b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Sorts by `key`; ties stay grouped by device in the device's sensor order (the input order).
fn sort_rows(rows: &mut [Row], key: SortKey, dir: Dir) {
    rows.sort_by(|a, b| {
        let primary = match key {
            SortKey::Status => dir.apply(rank(a.status).cmp(&rank(b.status))),
            SortKey::Sensor => dir.apply(natural_cmp(&a.name, &b.name)),
            SortKey::Device => dir.apply(natural_cmp(&a.device_name, &b.device_name)),
            SortKey::Value => missing_last(a.total, b.total, |x, y| {
                x.0.cmp(&y.0).then_with(|| dir.apply(x.1.total_cmp(&y.1)))
            }),
            SortKey::Since => missing_last(a.since_ms, b.since_ms, |x, y| dir.apply(x.cmp(y))),
        };
        primary
            .then_with(|| natural_cmp(&a.device_name, &b.device_name))
            .then_with(|| a.device_id.cmp(&b.device_id))
    });
}

/// The group path of every group, e.g. `Netzwerk / Switche`.
pub(crate) fn group_paths(groups: &[Group]) -> HashMap<i64, String> {
    groups
        .iter()
        .map(|g| {
            let mut names = vec![g.name.as_str()];
            let mut parent = g.parent_id;
            // Bounded by the number of groups, so a broken parent chain cannot loop.
            while let Some(id) = parent {
                match groups.iter().find(|p| p.id == id) {
                    Some(p) if names.len() < groups.len() => {
                        names.push(p.name.as_str());
                        parent = p.parent_id;
                    }
                    _ => break,
                }
            }
            names.reverse();
            (g.id, names.join(" / "))
        })
        .collect()
}

/// One status tile; the overview and the lists show the same tiles.
pub struct Tile {
    /// `all`, `up`, `warning`, `down` or `inactive` (chooses the symbol).
    pub kind: &'static str,
    pub label: String,
    pub count: usize,
    /// The line under the number, e.g. "7 Geräte" or "92 % aller Sensoren".
    pub sub: String,
    pub href: String,
    /// Status colour, plus `active` for warnings and failures that are there.
    pub class: String,
    /// The list being shown (never on the overview).
    pub current: bool,
}

impl Tile {
    fn class(filter: Filter, count: usize) -> String {
        match filter {
            Filter::All => String::new(),
            Filter::Up => " status-up".to_owned(),
            Filter::Warning | Filter::Down if count > 0 => {
                format!(" status-{} active", filter.as_str())
            }
            Filter::Warning | Filter::Down => format!(" status-{}", filter.as_str()),
            Filter::Inactive => " status-paused".to_owned(),
        }
    }
}

/// The tiles for sensor counts in the order of [`Filter::ALL`] (all, up, warning, down,
/// inactive/unknown), each linking to its list, limited to `group` when given; `devices` is the
/// number of devices in that scope (the line under "Sensoren").
pub fn tiles(
    t: Lang,
    counts: [usize; 5],
    devices: usize,
    current: Option<Filter>,
    group: Option<i64>,
) -> Vec<Tile> {
    let scope = group.map(|g| format!("?group={g}")).unwrap_or_default();
    let total = counts[0];
    let sub = |f: Filter, count: usize| match f {
        Filter::All => t.get1("count-devices", "count", devices),
        Filter::Up if total == 0 => t.get("no-sensors"),
        Filter::Up => t.get1("tile-share", "percent", share(count, total)),
        Filter::Warning if count == 0 => t.get("tile-warning-none"),
        Filter::Warning => t.get("tile-warning"),
        Filter::Down if count == 0 => t.get("tile-down-none"),
        Filter::Down => t.get("tile-down"),
        Filter::Inactive if count == 0 => t.get("tile-inactive-none"),
        Filter::Inactive => t.get("tile-inactive"),
    };
    Filter::ALL
        .into_iter()
        .zip(counts)
        .map(|(f, count)| Tile {
            sub: sub(f, count),
            kind: f.as_str(),
            // The first tile is "Sensoren" as on the overview; the page title says "Alle Sensoren".
            label: t.get(if f == Filter::All {
                "sensors"
            } else {
                f.label_id()
            }),
            count,
            href: format!("/status/{}{scope}", f.as_str()),
            class: Tile::class(f, count),
            current: current == Some(f),
        })
        .collect()
}

/// `part` as a whole-number percentage of `total`; 100 only when all are in it, 0 only when none are.
fn share(part: usize, total: usize) -> usize {
    match part {
        0 => 0,
        p if p >= total => 100,
        p => ((p * 100 + total / 2) / total).clamp(1, 99),
    }
}

pub struct Column {
    pub label: String,
    /// Sorts by this column (the message column is not sortable).
    pub href: Option<String>,
    /// `aria-sort` of the sorted column, otherwise empty.
    pub aria_sort: &'static str,
    pub right: bool,
}

#[derive(Template)]
#[template(path = "status_list.html")]
struct StatusListTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    title: String,
    /// The groups down to the one the list is limited to, for the breadcrumb.
    trail: Vec<crate::web::Crumb>,
    /// Path of that group, e.g. `Netzwerk / Switche` (empty for all sensors).
    group: String,
    /// The same list for all groups.
    all_href: String,
    tiles: Vec<Tile>,
    columns: Vec<Column>,
    rows: Vec<Row>,
    /// The search text (empty without a search).
    search: String,
    /// The same list without the search.
    clear_href: String,
}

#[derive(Deserialize)]
pub struct SortQuery {
    sort: Option<String>,
    dir: Option<String>,
    group: Option<i64>,
    q: Option<String>,
}

/// Whether a row contains every word of `words` (already in lower case) in its names, address, group or message.
fn row_matches(words: &[String], row: &Row, host: &str) -> bool {
    let text = format!(
        "{}\n{}\n{}\n{}\n{}",
        row.name, row.device_name, host, row.group, row.message
    )
    .to_lowercase();
    words.iter().all(|w| text.contains(w.as_str()))
}

pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(filter): Path<String>,
    Query(query): Query<SortQuery>,
) -> Result<Html<String>> {
    let filter = Filter::parse(&filter).ok_or(Error::NotFound)?;
    let (default_key, default_dir) = filter.default_sort();
    let key = query
        .sort
        .as_deref()
        .and_then(SortKey::parse)
        .unwrap_or(default_key);
    let dir = query
        .dir
        .as_deref()
        .and_then(Dir::parse)
        .unwrap_or(if key == default_key {
            default_dir
        } else {
            key.first_dir()
        });

    let search: String = query
        .q
        .as_deref()
        .unwrap_or_default()
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_SEARCH_CHARS)
        .collect();
    let words: Vec<String> = search.split_whitespace().map(str::to_lowercase).collect();

    let t = user.lang;
    let mibs = state.mib_index();
    let all_groups = groups::list_groups(&state.pool).await?;
    // The group the list is limited to, with its subgroups.
    let scope = match query.group {
        Some(id) if all_groups.iter().any(|g| g.id == id) => {
            Some((id, crate::devices::subtree_ids(&all_groups, id)))
        }
        Some(_) => return Err(Error::NotFound),
        None => None,
    };
    let paths = group_paths(&all_groups);
    let devices: HashMap<i64, Device> = devices::list_devices(&state.pool)
        .await?
        .into_iter()
        .map(|d| (d.id, d))
        .collect();
    let states = status::list_states(&state.pool).await?;
    let now = now_ms();
    let pauses = crate::storage::pauses::load_all(&state.pool, now).await?;

    let mut counts = [0usize; Filter::ALL.len()];
    let mut rows = Vec::new();
    for sensor in sensors::list_all_sensors(&state.pool).await? {
        let Some(device) = devices.get(&sensor.device_id) else {
            continue;
        };
        if let Some((_, ids)) = &scope
            && !device.group_id.is_some_and(|g| ids.contains(&g))
        {
            continue;
        }
        let sensor_state = states.get(&sensor.id);
        let why = pauses.sensor(sensor.id, device.id);
        let active = why.is_none();
        let current = crate::status::shown_status(!active, sensor_state.map(|s| s.status));
        for (n, f) in counts.iter_mut().zip(Filter::ALL) {
            if f.matches(current) {
                *n += 1;
            }
        }
        if !filter.matches(current) {
            continue;
        }
        // Since when a polled sensor is in its status; a paused one has no current status.
        let since_ms = sensor_state
            .filter(|s| active && s.status != Status::Paused)
            .map(|s| s.since_ms);
        let fresh = if active {
            Some(fresh_values(&state.pool, &sensor, device, &mibs, t, now).await?)
        } else {
            None
        };
        let (values, value_title, total) = fresh.map_or((Vec::new(), String::new(), None), |f| {
            (f.shown, f.title, f.total)
        });
        let rank = sensor
            .channels
            .first()
            .map_or(u8::MAX, |c| unit_rank(c.unit));
        let total = total.map(|v| (rank, v));
        let row = Row {
            id: sensor.id,
            name: sensor.name,
            device_id: device.id,
            device_name: device.name.clone(),
            group: device
                .group_id
                .and_then(|g| paths.get(&g).cloned())
                .unwrap_or_default(),
            status: current,
            status_label: t.get(current.label_id()),
            message: sensor_state
                .filter(|_| active)
                .map(|s| crate::status::shown(t, &s.message))
                .unwrap_or_default(),
            pause: why.as_ref().map(|w| crate::web::pause::note(t, w)),
            since: since_ms.map(time_html).unwrap_or_default(),
            duration: since_ms
                .map(|ms| format_value(t, Unit::Seconds, ((now - ms) / 1_000) as f64))
                .unwrap_or_default(),
            since_ms,
            values,
            value_title,
            total,
        };
        if words.is_empty() || row_matches(&words, &row, &device.host) {
            rows.push(row);
        }
    }
    sort_rows(&mut rows, key, dir);

    let group = scope.as_ref().map(|(id, _)| *id);
    let device_count = devices
        .values()
        .filter(|d| match &scope {
            Some((_, ids)) => d.group_id.is_some_and(|g| ids.contains(&g)),
            None => true,
        })
        .count();
    let tiles = tiles(t, counts, device_count, Some(filter), group);
    let mut group_param = group.map(|g| format!("group={g}&")).unwrap_or_default();
    let clear_href = format!(
        "/status/{}{}",
        filter.as_str(),
        group.map(|g| format!("?group={g}")).unwrap_or_default()
    );
    if !search.is_empty() {
        group_param.push_str(&format!("q={}&", url_param(&search)));
    }
    let mut columns: Vec<Column> = SortKey::ALL
        .into_iter()
        .map(|k| {
            let next = if k == key { dir.flip() } else { k.first_dir() };
            Column {
                label: t.get(k.label_id()),
                href: Some(format!(
                    "/status/{}?{group_param}sort={}&dir={}",
                    filter.as_str(),
                    k.as_str(),
                    next.as_str()
                )),
                aria_sort: if k == key { dir.aria_sort() } else { "" },
                right: k == SortKey::Value,
            }
        })
        .collect();
    // Status, sensor, device, message, value, since.
    columns.insert(
        3,
        Column {
            label: t.get("message"),
            href: None,
            aria_sort: "",
            right: false,
        },
    );
    let page = StatusListTemplate {
        layout: user.layout(),
        title: t.get(filter.label_id()),
        trail: crate::web::group_trail(&state.pool, group).await?,
        group: group
            .and_then(|g| paths.get(&g).cloned())
            .unwrap_or_default(),
        all_href: format!("/status/{}", filter.as_str()),
        tiles,
        columns,
        rows,
        search,
        clear_href,
        t,
    };
    Ok(Html(page.render()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The share under "In Ordnung" is 100 % or 0 % only when it is exactly so.
    #[test]
    fn shares_round_but_never_to_all_or_none() {
        assert_eq!(share(0, 0), 0);
        assert_eq!(share(0, 7), 0);
        assert_eq!(share(7, 7), 100);
        assert_eq!(share(10, 15), 67);
        assert_eq!(share(199, 200), 99);
        assert_eq!(share(1, 300), 1);
    }

    #[test]
    fn tiles_say_what_their_numbers_mean() {
        let sub = |counts, devices| -> Vec<String> {
            tiles(Lang::De, counts, devices, None, None)
                .into_iter()
                .map(|tile| tile.sub)
                .collect()
        };
        assert_eq!(
            sub([15, 10, 1, 3, 1], 7),
            [
                "7 Geräte",
                "67 % aller Sensoren",
                "Grenzwert überschritten",
                "keine Antwort oder Fehler",
                "von Hand oder per Zeitplan"
            ]
        );
        assert_eq!(
            sub([0, 0, 0, 0, 0], 1),
            ["1 Gerät", "keine Sensoren", "keine", "keiner", "keine"]
        );
    }

    fn row(
        id: i64,
        name: &str,
        device: &str,
        status: Status,
        since: Option<i64>,
        total: Option<(u8, f64)>,
    ) -> Row {
        Row {
            id,
            name: name.to_owned(),
            device_id: id,
            device_name: device.to_owned(),
            group: String::new(),
            status,
            status_label: String::new(),
            message: String::new(),
            pause: None,
            since_ms: since,
            since: String::new(),
            duration: String::new(),
            values: Vec::new(),
            value_title: String::new(),
            total,
        }
    }

    fn ids(rows: &[Row]) -> Vec<i64> {
        rows.iter().map(|r| r.id).collect()
    }

    #[test]
    fn rows_sort_by_every_column_with_missing_values_last() {
        let mut rows = vec![
            row(
                1,
                "Port 10",
                "Switch 2",
                Status::Up,
                Some(300),
                Some((0, 5.0)),
            ),
            row(2, "Port 9", "Switch 10", Status::Down, Some(100), None),
            row(3, "Ping", "AP 1", Status::Warning, None, Some((0, 50.0))),
            row(
                4,
                "Uptime",
                "Switch 2",
                Status::Paused,
                Some(200),
                Some((5, 9e6)),
            ),
        ];
        sort_rows(&mut rows, SortKey::Status, Dir::Asc);
        assert_eq!(ids(&rows), [2, 3, 1, 4], "worst first, inactive last");
        sort_rows(&mut rows, SortKey::Sensor, Dir::Asc);
        assert_eq!(ids(&rows), [3, 2, 1, 4], "numbers by size");
        sort_rows(&mut rows, SortKey::Device, Dir::Desc);
        assert_eq!(ids(&rows), [2, 1, 4, 3]);
        sort_rows(&mut rows, SortKey::Value, Dir::Desc);
        assert_eq!(ids(&rows), [3, 1, 4, 2], "traffic first, then uptime");
        sort_rows(&mut rows, SortKey::Value, Dir::Asc);
        assert_eq!(
            ids(&rows),
            [1, 3, 4, 2],
            "units keep their order, no value stays last"
        );
        sort_rows(&mut rows, SortKey::Since, Dir::Desc);
        assert_eq!(ids(&rows), [1, 4, 2, 3]);
        sort_rows(&mut rows, SortKey::Since, Dir::Asc);
        assert_eq!(ids(&rows), [2, 4, 1, 3]);
    }

    #[test]
    fn filters_split_like_the_tiles() {
        let all = [
            Status::Up,
            Status::Warning,
            Status::Down,
            Status::Unknown,
            Status::Paused,
        ];
        let count = |f: Filter| all.iter().filter(|s| f.matches(**s)).count();
        assert_eq!(
            Filter::ALL.map(count),
            [5, 1, 1, 1, 2],
            "all, up, warning, down, inactive/unknown"
        );
        assert_eq!(Filter::parse("down"), Some(Filter::Down));
        assert_eq!(Filter::parse("paused"), None);
    }

    #[tokio::test]
    async fn tiles_open_sortable_lists_of_their_sensors() {
        use crate::devices::{DeviceInput, Polling};
        use crate::sensors::spec::SensorSpec;
        use crate::status::Assessment;
        use crate::storage::sensors::{create_sensor, set_enabled};
        use crate::storage::status::record_poll;
        use crate::web::testutil::*;

        let h = harness(true).await;
        let group = crate::storage::groups::create_group(&h.state.pool, "Netzwerk", None)
            .await
            .unwrap();
        let device = devices::save_device(
            &h.state.pool,
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
        let pool = &h.state.pool;
        let ok = create_sensor(pool, device, "Ping", &SensorSpec::ping())
            .await
            .unwrap();
        let down = create_sensor(pool, device, "Gateway", &SensorSpec::ping())
            .await
            .unwrap();
        let paused = create_sensor(pool, device, "Alt", &SensorSpec::ping())
            .await
            .unwrap();
        set_enabled(pool, paused, false).await.unwrap();
        let now = now_ms();
        record_poll(pool, ok, now, &[], &Assessment::Healthy("OK".into()))
            .await
            .unwrap();
        for ts in [now - 120_000, now - 60_000] {
            record_poll(pool, down, ts, &[], &Assessment::Failed("weg".into()))
                .await
                .unwrap();
        }
        let (viewer, _) = login(&h, "bert").await;

        let home = send(&h.app, get("/", Some(&viewer))).await;
        assert!(
            home.body
                .contains("<a class=\"tile status-down active\" href=\"/status/down\">")
        );

        let page = send(&h.app, get("/status/down", Some(&viewer))).await;
        assert_eq!(page.status, axum::http::StatusCode::OK);
        // Overview and status lists can be refreshed, also by viewers.
        for body in [&home.body, &page.body] {
            assert!(body.contains("data-refresh-now title=\"Aktualisieren\""));
        }
        assert!(page.body.contains(&format!("href=\"/sensors/{down}\"")));
        assert!(!page.body.contains(&format!("href=\"/sensors/{ok}\"")));
        assert!(page.body.contains("weg") && page.body.contains("Switch &#60;1&#62;"));
        assert!(!page.body.contains("Switch <1>"), "escaped");
        assert!(page.body.contains("<span class=\"sub\">Netzwerk</span>"));
        assert!(page.body.contains("<time class=\"ts\""), "since when");
        // Default: newest change first; a click on the column turns it round.
        assert!(page.body.contains(
            "aria-sort=\"descending\"><a class=\"sort\" href=\"/status/down?sort=since&#38;dir=asc\">Seit</a>"
        ));
        // The same tiles as on the overview, the shown list marked.
        assert!(page.body.contains(
            "<a class=\"tile status-down active\" href=\"/status/down\" aria-current=\"page\"><div class=\"tile-label\">"
        ));
        assert!(
            page.body
                .contains("Pausiert / unbekannt</div><div class=\"tile-value\">1</div><div class=\"tile-sub\">von Hand oder per Zeitplan</div></a>")
        );
        assert!(
            !home
                .body
                .contains("aria-current=\"page\"><div class=\"tile-label")
        );

        let all = send(
            &h.app,
            get("/status/all?sort=sensor&dir=asc", Some(&viewer)),
        )
        .await;
        let pos = |id: i64| all.body.find(&format!("href=\"/sensors/{id}\"")).unwrap();
        assert!(
            pos(paused) < pos(down) && pos(down) < pos(ok),
            "Alt, Gateway, Ping"
        );

        // The search of the header: every word in the names, address, group or message, any case.
        let found = send(&h.app, get("/status/all?q=SWITCH%20weg", Some(&viewer))).await;
        assert!(found.body.contains(&format!("href=\"/sensors/{down}\"")));
        assert!(!found.body.contains(&format!("href=\"/sensors/{ok}\"")));
        assert!(!found.body.contains(&format!("href=\"/sensors/{paused}\"")));
        assert!(found.body.contains("Suche „SWITCH weg“"));
        assert!(
            found.body.contains("q=SWITCH%20weg&#38;sort=status"),
            "sorting keeps the search"
        );
        let by_address = send(&h.app, get("/status/all?q=192.0.2.1", Some(&viewer))).await;
        assert!(by_address.body.contains("3 Sensoren"));

        // Needs attention on the overview: the failed sensor with its message.
        assert!(home.body.contains("Braucht Aufmerksamkeit"));
        assert!(home.body.contains("1 von 3 Sensoren"));
        // The counters of the header, for pages the browser opens.
        let mut request = get("/log", Some(&viewer));
        request
            .headers_mut()
            .insert(axum::http::header::ACCEPT, "text/html".parse().unwrap());
        let log = send(&h.app, request).await.body;
        assert!(log.contains("<a class=\"up\" href=\"/status/up\""), "{log}");
        assert!(log.contains("<a class=\"warn zero\" href=\"/status/warning\""));
        assert!(log.contains("<a class=\"down\" href=\"/status/down\""));
        assert!(
            !home.body.contains("class=\"status-chip\""),
            "not for scripts"
        );
        let inactive = send(&h.app, get("/status/inactive", Some(&viewer))).await;
        assert!(
            inactive
                .body
                .contains(&format!("href=\"/sensors/{paused}\""))
        );
        let empty = send(&h.app, get("/status/warning", Some(&viewer))).await;
        assert!(empty.body.contains("Keine Sensoren mit diesem Status."));

        let unknown = send(&h.app, get("/status/kaputt", Some(&viewer))).await;
        assert_eq!(unknown.status, axum::http::StatusCode::NOT_FOUND);
        let anonymous = send(&h.app, get("/status/all", None)).await;
        assert!(anonymous.status.is_redirection());
    }

    #[test]
    fn group_paths_start_at_the_top_level() {
        let g = |id, name: &str, parent| Group {
            id,
            name: name.to_owned(),
            parent_id: parent,
        };
        let paths = group_paths(&[
            g(1, "Netzwerk", None),
            g(2, "Switche", Some(1)),
            g(3, "Rack", Some(2)),
            g(4, "Kreis", Some(4)),
        ]);
        assert_eq!(paths[&3], "Netzwerk / Switche / Rack");
        assert_eq!(paths[&1], "Netzwerk");
        assert!(paths[&4].starts_with("Kreis"), "a broken chain ends");
    }
}
