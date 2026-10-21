//! The start page: status tiles and all groups with their devices.

use std::collections::HashMap;

use askama::Template;
use axum::extract::{Query, State};
use axum::response::Html;
use serde::Deserialize;

use crate::devices::{Device, Group};
use crate::error::Result;
use crate::i18n::Lang;
use crate::pause::{Pauses, Source, Why};
use crate::status::{Status, device_status, shown_status};
use crate::storage::sensors::SensorRecord;
use crate::storage::{devices, groups, pauses, sensors, status};
use crate::web::auth::CurrentUser;
use crate::web::pause::{PauseNote, note};
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

    /// The numbers of the status tiles: all, up, warning, down, inactive/unknown.
    fn tile_counts(&self) -> [usize; 5] {
        [
            self.total(),
            self.up,
            self.warning,
            self.down,
            self.paused + self.unknown,
        ]
    }

    /// Active sensors (inactive ones are not polled).
    pub fn active(&self) -> usize {
        self.total() - self.paused
    }

    /// The status labels of a tree row, worst first: one per status with its count, inactive sensors
    /// last. A lone active status (or only inactive sensors) reads "N Sensoren" like in PRTG.
    fn pills(&self, t: Lang) -> Vec<Pill> {
        let active: Vec<_> = [
            (self.down, "down", "✕", "count-down"),
            (self.warning, "warning", "!", "count-warning"),
            (self.unknown, "unknown", "?", "count-unknown"),
            (self.up, "up", "✓", "count-up"),
        ]
        .into_iter()
        .filter(|(n, ..)| *n > 0)
        .collect();
        // "N Sensoren" for a lone active status, or for inactive sensors when nothing else is there.
        let lone_active = active.len() == 1;
        let lone_paused = active.is_empty();
        let paused = (self.paused > 0).then_some((self.paused, "paused", "‖", "count-inactive"));
        active
            .into_iter()
            .map(|k| (k, lone_active))
            .chain(paused.map(|k| (k, lone_paused)))
            .map(|((n, class, glyph, title), lone)| Pill {
                class,
                glyph,
                text: if lone {
                    t.get1("count-sensors", "count", n)
                } else {
                    n.to_string()
                },
                title: t.get1(title, "count", n),
            })
            .collect()
    }
}

/// A status label of a tree row (`text` is the count, or "N Sensoren" when it stands alone).
pub struct Pill {
    pub class: &'static str,
    pub glyph: &'static str,
    pub text: String,
    pub title: String,
}

pub struct DeviceCard {
    pub device: Device,
    pub counts: Counts,
    pub pills: Vec<Pill>,
    pub pause: Option<PauseNote>,
    /// Paused by itself, so it can be resumed here (not only through a group).
    pub paused_itself: bool,
}

/// The devices without a group (`Ohne Gruppe`, or `Alle Geräte` when there are no groups).
pub struct Ungrouped {
    /// Id 0, like the order page of this level.
    pub summary: Summary,
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
    pub pills: Vec<Pill>,
    pub status_class: &'static str,
    pub pause: Option<PauseNote>,
    pub paused_itself: bool,
}

impl Summary {
    fn new(
        t: Lang,
        id: i64,
        name: String,
        devices: usize,
        counts: Counts,
        why: Option<Why>,
    ) -> Self {
        let status = counts.status();
        Self {
            paused_itself: why.as_ref().is_some_and(|w| w.source == Source::Own),
            pause: why.as_ref().map(|w| note(t, w)),
            id,
            name,
            devices,
            pills: counts.pills(t),
            status_class: status.map_or("unknown", Status::as_str),
            counts,
        }
    }
}

/// A top-level group: its own devices first, then its subgroups.
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
    tiles: Vec<crate::web::status_list::Tile>,
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

/// Effective status of a sensor: paused when it, its device or a group is paused, unknown before
/// the first poll.
fn effective(
    sensor: &SensorRecord,
    states: &HashMap<i64, crate::status::SensorState>,
    pauses: &Pauses,
) -> Status {
    shown_status(
        pauses.sensor(sensor.id, sensor.device_id).is_some(),
        states.get(&sensor.id).map(|s| s.status),
    )
}

/// Appends the rows of `group`'s subtree (its devices, then each subgroup wrapped in Start/End)
/// and returns the subtree's device count and sensor counts.
fn subtree(
    t: Lang,
    pauses: &Pauses,
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
        let (n, c) = subtree(
            t,
            pauses,
            Some(child.id),
            groups,
            cards,
            depth + 1,
            &mut inner,
        );
        if inner.is_empty() {
            inner.push(Row::Empty);
        }
        rows.push(Row::Start(Summary::new(
            t,
            child.id,
            child.name.clone(),
            n,
            c,
            pauses.group(child.id),
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
    let pauses = pauses::load_all(&state.pool, crate::web::sensors::now_ms()).await?;

    let mut totals = Counts::default();
    let mut devices_down = 0;
    // Device cards by group, in display order.
    let mut cards: HashMap<Option<i64>, Vec<DeviceCard>> = HashMap::new();
    for d in &devices {
        let own: Vec<&SensorRecord> = all_sensors.iter().filter(|s| s.device_id == d.id).collect();
        let mut counts = Counts::default();
        for s in &own {
            counts.add(effective(s, &states, &pauses));
        }
        totals.merge(&counts);
        let overall = device_status(own.iter().map(|s| {
            (
                pauses.sensor(s.id, d.id).is_none(),
                states.get(&s.id).map(|st| st.status),
            )
        }));
        let why = pauses.device(d.id);
        if overall == Status::Down {
            devices_down += 1;
        }
        cards.entry(d.group_id).or_default().push(DeviceCard {
            device: d.clone(),
            pills: counts.pills(user.lang),
            paused_itself: why.as_ref().is_some_and(|w| w.source == Source::Own),
            pause: why.as_ref().map(|w| note(user.lang, w)),
            counts,
        });
    }

    let ungrouped = cards.remove(&None).map(|devices| {
        let mut counts = Counts::default();
        for card in &devices {
            counts.merge(&card.counts);
        }
        let name = user.lang.get(if groups.is_empty() {
            "overview-all-devices"
        } else {
            "overview-ungrouped"
        });
        Ungrouped {
            summary: Summary::new(user.lang, 0, name, devices.len(), counts, None),
            devices,
        }
    });
    let top: Vec<&Group> = groups.iter().filter(|g| g.parent_id.is_none()).collect();
    let tops = top
        .iter()
        .map(|g| {
            let mut rows = Vec::new();
            let (n, counts) = subtree(
                user.lang,
                &pauses,
                Some(g.id),
                &groups,
                &mut cards,
                1,
                &mut rows,
            );
            TopGroup {
                summary: Summary::new(
                    user.lang,
                    g.id,
                    g.name.clone(),
                    n,
                    counts,
                    pauses.group(g.id),
                ),
                rows,
            }
        })
        .collect();

    let page = HomeTemplate {
        layout: user.layout(),
        t: user.lang,
        ungrouped,
        groups: tops,
        tiles: crate::web::status_list::tiles(user.lang, totals.tile_counts(), None),
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
    fn pills_count_each_status_worst_first_and_inactive_last() {
        let text = |counts: Counts| -> Vec<String> {
            counts
                .pills(Lang::De)
                .iter()
                .map(|p| format!("{} {} {} ({})", p.class, p.glyph, p.text, p.title))
                .collect()
        };
        let mixed = Counts {
            up: 8,
            warning: 1,
            down: 2,
            unknown: 0,
            paused: 5,
        };
        assert_eq!(
            text(mixed),
            [
                "down ✕ 2 (2 Ausfall)",
                "warning ! 1 (1 Warnung)",
                "up ✓ 8 (8 in Ordnung)",
                "paused ‖ 5 (5 pausiert)"
            ]
        );
        assert_eq!((mixed.total(), mixed.active()), (16, 11));
        let healthy = Counts {
            up: 9,
            paused: 2,
            ..Counts::default()
        };
        assert_eq!(
            text(healthy),
            ["up ✓ 9 Sensoren (9 in Ordnung)", "paused ‖ 2 (2 pausiert)"]
        );
        let prepared = Counts {
            paused: 1,
            ..Counts::default()
        };
        assert_eq!(text(prepared), ["paused ‖ 1 Sensor (1 pausiert)"]);
        assert!(Counts::default().pills(Lang::De).is_empty());
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
                kind: Default::default(),
            },
            pills: Vec::new(),
            pause: None,
            paused_itself: false,
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
        let (n, counts) = subtree(
            Lang::De,
            &Pauses::default(),
            Some(1),
            &groups,
            &mut cards,
            1,
            &mut rows,
        );
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
