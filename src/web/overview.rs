//! The start page: status tiles and all groups with their devices.

use std::collections::HashMap;

use askama::Template;
use axum::extract::{Query, State};
use axum::response::Html;
use serde::Deserialize;

use crate::devices::{Device, Group, flatten_groups};
use crate::error::Result;
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
    pub status_label: &'static str,
    pub status_class: &'static str,
    pub counts: Counts,
    pub segments: Vec<Segment>,
}

/// A group (or the ungrouped devices, `id == 0`) with its devices.
pub struct Section {
    pub id: i64,
    pub name: String,
    /// Names of the parent groups, outermost first.
    pub parents: Vec<String>,
    pub depth: usize,
    pub devices: Vec<DeviceCard>,
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate {
    layout: Layout,
    sections: Vec<Section>,
    totals: Counts,
    device_count: usize,
    devices_down: usize,
    /// Groups at the top level; their order is only worth a button when there are several.
    top_groups: usize,
    flash: Option<&'static str>,
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

fn parents_of(groups: &[Group], group: &Group) -> Vec<String> {
    let by_id: HashMap<i64, &Group> = groups.iter().map(|g| (g.id, g)).collect();
    let mut names = Vec::new();
    let mut current = group.parent_id;
    while let Some(id) = current {
        let Some(parent) = by_id.get(&id) else { break };
        names.push(parent.name.clone());
        current = parent.parent_id;
        if names.len() > crate::devices::MAX_GROUP_DEPTH {
            break; // corrupted data must not loop
        }
    }
    names.reverse();
    names
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
    let card = |d: &Device, totals: &mut Counts, devices_down: &mut usize| {
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
            *devices_down += 1;
        }
        DeviceCard {
            device: d.clone(),
            status_label: if own.is_empty() {
                "keine Sensoren"
            } else {
                overall.label()
            },
            status_class: overall.as_str(),
            segments: counts.segments(),
            counts,
        }
    };

    let mut sections = Vec::new();
    let ungrouped: Vec<DeviceCard> = devices
        .iter()
        .filter(|d| d.group_id.is_none())
        .map(|d| card(d, &mut totals, &mut devices_down))
        .collect();
    if !ungrouped.is_empty() {
        sections.push(Section {
            id: 0,
            name: if groups.is_empty() {
                "Alle Geräte".to_owned()
            } else {
                "Ohne Gruppe".to_owned()
            },
            parents: Vec::new(),
            depth: 1,
            devices: ungrouped,
        });
    }
    for node in flatten_groups(&groups) {
        let members = devices
            .iter()
            .filter(|d| d.group_id == Some(node.group.id))
            .map(|d| card(d, &mut totals, &mut devices_down))
            .collect();
        sections.push(Section {
            id: node.group.id,
            parents: parents_of(&groups, &node.group),
            name: node.group.name.clone(),
            depth: node.depth,
            devices: members,
        });
    }

    let page = HomeTemplate {
        layout: user.layout(),
        sections,
        totals,
        device_count: devices.len(),
        devices_down,
        top_groups: groups.iter().filter(|g| g.parent_id.is_none()).count(),
        flash: flash_message(query.msg.as_deref()),
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

    #[test]
    fn parent_paths_are_outermost_first() {
        let g = |id, name: &str, parent| Group {
            id,
            name: name.to_owned(),
            parent_id: parent,
        };
        let groups = [
            g(1, "Haus", None),
            g(2, "Etage", Some(1)),
            g(3, "Rack", Some(2)),
        ];
        assert_eq!(parents_of(&groups, &groups[2]), ["Haus", "Etage"]);
        assert!(parents_of(&groups, &groups[0]).is_empty());
    }
}
