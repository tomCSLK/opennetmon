//! The start page: status tiles and all groups with their devices.

use std::collections::HashMap;

use askama::Template;
use axum::extract::{Query, State};
use axum::response::Html;
use serde::Deserialize;

use crate::devices::{Device, Group};
use crate::error::Result;
use crate::i18n::Lang;
use crate::status::{Status, device_status};
use crate::storage::sensors::SensorRecord;
use crate::storage::{devices, groups, sensors, status};
use crate::web::auth::CurrentUser;
use crate::web::{AppState, Layout, flash_message};

/// Number of sensors per effective status (disabled sensors count as paused).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub up: usize,
    pub warning: usize,
    pub down: usize,
    pub unknown: usize,
    pub paused: usize,
}

impl Counts {
    pub fn total(&self) -> usize {
        self.up + self.warning + self.down + self.unknown + self.paused
    }

    fn add(&mut self, status: Status) {
        match status {
            Status::Up => self.up += 1,
            Status::Warning => self.warning += 1,
            Status::Down => self.down += 1,
            Status::Unknown => self.unknown += 1,
            Status::Paused => self.paused += 1,
        }
    }

    fn merge(&mut self, other: &Counts) {
        self.up += other.up;
        self.warning += other.warning;
        self.down += other.down;
        self.unknown += other.unknown;
        self.paused += other.paused;
    }

    /// Active sensors (inactive ones are not polled).
    pub fn active(&self) -> usize {
        self.total() - self.paused
    }

    /// Segments of the stacked status bar, worst first; `x`/`width` in percent. Inactive sensors
    /// are left out, so prepared but unused sensors do not grey out a healthy device.
    fn segments(&self) -> Vec<Segment> {
        let total = self.active();
        if total == 0 {
            return Vec::new();
        }
        let mut x = 0.0;
        [
            (self.down, "seg-down"),
            (self.warning, "seg-warning"),
            (self.unknown, "seg-unknown"),
            (self.up, "seg-up"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, class)| {
            let width = n as f64 * 100.0 / total as f64;
            let segment = Segment {
                x: format!("{x:.3}%"),
                width: format!("{width:.3}%"),
                class,
            };
            x += width;
            segment
        })
        .collect()
    }
}

pub struct Segment {
    pub x: String,
    pub width: String,
    pub class: &'static str,
}

pub struct DeviceCard {
    pub device: Device,
    pub status_label: String,
    pub status_class: &'static str,
    pub counts: Counts,
    pub segments: Vec<Segment>,
}

/// The devices without a group (`Ohne Gruppe`, or `Alle Geräte` when there are no groups).
pub struct Ungrouped {
    pub name: String,
    pub devices: Vec<DeviceCard>,
}

/// One line inside a top-level group card. Subgroups are opened by `Start` and closed by `End`, so
/// the template renders a nested tree from a flat list.
pub enum Row {
    Start(Summary),
    Device(DeviceCard),
    /// A subgroup with neither devices nor subgroups.
    Empty,
    End,
}

/// A group with the totals of its whole subtree.
pub struct Summary {
    pub id: i64,
    pub name: String,
    /// Devices in the group and all its subgroups.
    pub devices: usize,
    pub counts: Counts,
    pub segments: Vec<Segment>,
    pub status_label: String,
    pub status_class: &'static str,
}

impl Summary {
    fn new(t: Lang, id: i64, name: String, devices: usize, counts: Counts) -> Self {
        let status = counts.status();
        Self {
            id,
            name,
            devices,
            segments: counts.segments(),
            status_label: t.get(status.map_or("no-sensors", Status::label_id)),
            status_class: status.map_or("unknown", Status::as_str),
            counts,
        }
    }
}

/// A top-level group: one card with its own devices first, then its subgroups.
pub struct TopGroup {
    pub summary: Summary,
    pub rows: Vec<Row>,
}

impl Counts {
    /// Worst state of the active sensors; `None` without active sensors.
    fn status(&self) -> Option<Status> {
        [
            (self.down, Status::Down),
            (self.warning, Status::Warning),
            (self.unknown, Status::Unknown),
            (self.up, Status::Up),
        ]
        .into_iter()
        .find(|(n, _)| *n > 0)
        .map(|(_, s)| s)
    }
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    ungrouped: Option<Ungrouped>,
    groups: Vec<TopGroup>,
    totals: Counts,
    device_count: usize,
    devices_down: usize,
    /// Groups at the top level; their order is only worth a button when there are several.
    top_groups: usize,
    flash: Option<String>,
}

#[derive(Deserialize)]
pub struct FlashQuery {
    pub msg: Option<String>,
}

/// Effective status of a sensor: paused when disabled, unknown before the first poll.
fn effective(sensor: &SensorRecord, states: &HashMap<i64, crate::status::SensorState>) -> Status {
    if !sensor.enabled {
        return Status::Paused;
    }
    states.get(&sensor.id).map_or(Status::Unknown, |s| s.status)
}

/// Appends the rows of `group`'s subtree (its devices, then each subgroup wrapped in Start/End)
/// and returns the subtree's device count and sensor counts.
fn subtree(
    t: Lang,
    group: Option<i64>,
    groups: &[Group],
    cards: &mut HashMap<Option<i64>, Vec<DeviceCard>>,
    depth: usize,
    rows: &mut Vec<Row>,
) -> (usize, Counts) {
    let mut devices = 0;
    let mut counts = Counts::default();
    for card in cards.remove(&group).unwrap_or_default() {
        devices += 1;
        counts.merge(&card.counts);
        rows.push(Row::Device(card));
    }
    if depth > crate::devices::MAX_GROUP_DEPTH {
        return (devices, counts); // corrupted data must not recurse forever
    }
    for child in groups
        .iter()
        .filter(|g| g.parent_id == group && Some(g.id) != group)
    {
        let mut inner = Vec::new();
        let (n, c) = subtree(t, Some(child.id), groups, cards, depth + 1, &mut inner);
        if inner.is_empty() {
            inner.push(Row::Empty);
        }
        rows.push(Row::Start(Summary::new(
            t,
            child.id,
            child.name.clone(),
            n,
            c,
        )));
        rows.extend(inner);
        rows.push(Row::End);
        devices += n;
        counts.merge(&c);
    }
    (devices, counts)
}

pub async fn home(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<FlashQuery>,
) -> Result<Html<String>> {
    let groups = groups::list_groups(&state.pool).await?;
    let devices = devices::list_devices(&state.pool).await?;
    let all_sensors = sensors::list_all_sensors(&state.pool).await?;
    let states = status::list_states(&state.pool).await?;

    let mut totals = Counts::default();
    let mut devices_down = 0;
    // Device cards by group, in display order.
    let mut cards: HashMap<Option<i64>, Vec<DeviceCard>> = HashMap::new();
    for d in &devices {
        let own: Vec<&SensorRecord> = all_sensors.iter().filter(|s| s.device_id == d.id).collect();
        let mut counts = Counts::default();
        for s in &own {
            counts.add(effective(s, &states));
        }
        totals.merge(&counts);
        let overall = device_status(
            own.iter()
                .map(|s| (s.enabled, states.get(&s.id).map(|st| st.status))),
        );
        if overall == Status::Down {
            devices_down += 1;
        }
        cards.entry(d.group_id).or_default().push(DeviceCard {
            device: d.clone(),
            status_label: user.lang.get(if own.is_empty() {
                "no-sensors"
            } else {
                overall.label_id()
            }),
            status_class: overall.as_str(),
            segments: counts.segments(),
            counts,
        });
    }

    let ungrouped = cards.remove(&None).map(|devices| Ungrouped {
        name: user.lang.get(if groups.is_empty() {
            "overview-all-devices"
        } else {
            "overview-ungrouped"
        }),
        devices,
    });
    let top: Vec<&Group> = groups.iter().filter(|g| g.parent_id.is_none()).collect();
    let tops = top
        .iter()
        .map(|g| {
            let mut rows = Vec::new();
            let (n, counts) = subtree(user.lang, Some(g.id), &groups, &mut cards, 1, &mut rows);
            TopGroup {
                summary: Summary::new(user.lang, g.id, g.name.clone(), n, counts),
                rows,
            }
        })
        .collect();

    let page = HomeTemplate {
        layout: user.layout(),
        t: user.lang,
        ungrouped,
        groups: tops,
        totals,
        device_count: devices.len(),
        devices_down,
        top_groups: top.len(),
        flash: flash_message(user.lang, query.msg.as_deref()),
    };
    Ok(Html(page.render()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_are_worst_first_and_add_up_to_the_whole_bar() {
        let counts = Counts {
            up: 8,
            warning: 1,
            down: 1,
            unknown: 0,
            paused: 5,
        };
        let segments = counts.segments();
        let classes: Vec<_> = segments.iter().map(|s| s.class).collect();
        assert_eq!(
            classes,
            ["seg-down", "seg-warning", "seg-up"],
            "inactive sensors are not in the bar"
        );
        assert_eq!(segments[0].x, "0.000%");
        assert_eq!(segments[0].width, "10.000%");
        assert_eq!(segments[2].x, "20.000%");
        assert_eq!(segments[2].width, "80.000%");
        assert_eq!((counts.total(), counts.active()), (15, 10));
        assert!(
            Counts {
                paused: 3,
                ..Counts::default()
            }
            .segments()
            .is_empty()
        );
        assert!(Counts::default().segments().is_empty());
    }

    fn g(id: i64, name: &str, parent: Option<i64>) -> Group {
        Group {
            id,
            name: name.to_owned(),
            parent_id: parent,
        }
    }

    fn card(id: i64, group: Option<i64>, counts: Counts) -> DeviceCard {
        DeviceCard {
            device: Device {
                id,
                name: format!("D{id}"),
                host: "192.0.2.1".to_owned(),
                group_id: group,
                profile_id: None,
                profile_name: None,
                polling: crate::devices::Polling::default(),
            },
            status_label: String::new(),
            status_class: "",
            segments: Vec::new(),
            counts,
        }
    }

    #[test]
    fn subgroups_nest_inside_their_group_and_add_up() {
        let groups = [
            g(1, "Netzwerk", None),
            g(2, "Switche", Some(1)),
            g(3, "Leer", Some(1)),
            g(4, "Rack", Some(2)),
        ];
        let up = Counts {
            up: 2,
            ..Counts::default()
        };
        let down = Counts {
            down: 1,
            up: 1,
            ..Counts::default()
        };
        let mut cards: HashMap<Option<i64>, Vec<DeviceCard>> = HashMap::new();
        cards
            .entry(Some(1))
            .or_default()
            .push(card(10, Some(1), up));
        cards
            .entry(Some(2))
            .or_default()
            .push(card(11, Some(2), up));
        cards
            .entry(Some(4))
            .or_default()
            .push(card(12, Some(4), down));
        let mut rows = Vec::new();
        let (n, counts) = subtree(Lang::De, Some(1), &groups, &mut cards, 1, &mut rows);
        assert_eq!((n, counts.status()), (3, Some(Status::Down)));
        let shape: Vec<String> = rows
            .iter()
            .map(|r| match r {
                Row::Start(s) => format!("[{} {} {}", s.name, s.devices, s.status_class),
                Row::Device(c) => c.device.name.clone(),
                Row::Empty => "-".to_owned(),
                Row::End => "]".to_owned(),
            })
            .collect();
        assert_eq!(
            shape,
            [
                "D10",
                "[Switche 2 down",
                "D11",
                "[Rack 1 down",
                "D12",
                "]",
                "]",
                "[Leer 0 unknown",
                "-",
                "]"
            ]
        );
        assert!(cards.is_empty(), "every card is placed once");
    }
}
