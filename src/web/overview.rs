//! The start page (status tiles and all groups with their devices) and the page of a group (the
//! same for the group's subtree).

use std::collections::HashMap;

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::response::Html;
use serde::Deserialize;

use crate::devices::{Device, Group};
use crate::error::{Error, Result};
use crate::i18n::Lang;
use crate::pause::{Pauses, Source, Why};
use crate::status::{Status, device_status, shown_status};
use crate::storage::sensors::SensorRecord;
use crate::storage::{devices, groups, pauses, sensors, status};
use crate::web::auth::CurrentUser;
use crate::web::pause::{PauseNote, note};
use crate::web::{AppState, Crumb, Layout, flash_message, group_trail};

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
    /// The device is down (counted in the page subtitle).
    pub down: bool,
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

/// Every device as a card, by group in display order, with what the pages count.
struct Forest {
    groups: Vec<Group>,
    cards: HashMap<Option<i64>, Vec<DeviceCard>>,
    pauses: Pauses,
    totals: Counts,
    device_count: usize,
    devices_down: usize,
}

async fn load(state: &AppState, t: Lang) -> Result<Forest> {
    let groups = groups::list_groups(&state.pool).await?;
    let devices = devices::list_devices(&state.pool).await?;
    let all_sensors = sensors::list_all_sensors(&state.pool).await?;
    let states = status::list_states(&state.pool).await?;
    let pauses = pauses::load_all(&state.pool, crate::web::sensors::now_ms()).await?;

    let mut totals = Counts::default();
    let mut devices_down = 0;
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
        let down = overall == Status::Down;
        if down {
            devices_down += 1;
        }
        cards.entry(d.group_id).or_default().push(DeviceCard {
            device: d.clone(),
            pills: counts.pills(t),
            paused_itself: why.as_ref().is_some_and(|w| w.source == Source::Own),
            pause: why.as_ref().map(|w| note(t, w)),
            down,
            counts,
        });
    }
    Ok(Forest {
        groups,
        cards,
        pauses,
        totals,
        device_count: devices.len(),
        devices_down,
    })
}

pub async fn home(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<FlashQuery>,
) -> Result<Html<String>> {
    let Forest {
        groups,
        mut cards,
        pauses,
        totals,
        device_count,
        devices_down,
    } = load(&state, user.lang).await?;

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
        tiles: crate::web::status_list::tiles(user.lang, totals.tile_counts(), None, None),
        totals,
        device_count,
        devices_down,
        top_groups: top.len(),
        flash: flash_message(user.lang, query.msg.as_deref()),
    };
    Ok(Html(page.render()?))
}

#[derive(Template)]
#[template(path = "group.html")]
struct GroupTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    /// The group with the totals of its subtree.
    summary: Summary,
    status_class: &'static str,
    status_label: String,
    /// The groups above, for the breadcrumb.
    trail: Vec<Crumb>,
    rows: Vec<Row>,
    tiles: Vec<crate::web::status_list::Tile>,
    devices_down: usize,
    /// `back` of the edit and pause pages opened here, so they return to this page.
    back: String,
    flash: Option<String>,
}

/// The page of a group: its devices and subgroups as on the overview, with tiles for its subtree.
pub async fn group(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<FlashQuery>,
) -> Result<Html<String>> {
    let t = user.lang;
    let Forest {
        groups,
        mut cards,
        pauses,
        ..
    } = load(&state, t).await?;
    let group = groups
        .iter()
        .find(|g| g.id == id)
        .ok_or(Error::NotFound)?
        .clone();
    let mut rows = Vec::new();
    let (n, counts) = subtree(t, &pauses, Some(id), &groups, &mut cards, 1, &mut rows);
    let devices_down = rows
        .iter()
        .filter(|r| matches!(r, Row::Device(card) if card.down))
        .count();
    let summary = Summary::new(t, id, group.name, n, counts, pauses.group(id));
    let status = if summary.pause.is_some() {
        Status::Paused
    } else {
        counts.status().unwrap_or(if counts.paused > 0 {
            Status::Paused
        } else {
            Status::Unknown
        })
    };
    let page = GroupTemplate {
        layout: user.layout(),
        status_class: status.as_str(),
        status_label: t.get(status.label_id()),
        trail: group_trail(&state.pool, group.parent_id).await?,
        tiles: crate::web::status_list::tiles(t, counts.tile_counts(), None, Some(id)),
        rows,
        summary,
        devices_down,
        back: format!("group-{id}"),
        flash: flash_message(t, query.msg.as_deref()),
        t,
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
            down: false,
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

    #[tokio::test]
    async fn a_group_page_shows_its_subtree_and_its_forms_return_there() {
        use axum::http::StatusCode;

        use crate::devices::{DeviceInput, Polling};
        use crate::sensors::spec::SensorSpec;
        use crate::status::Assessment;
        use crate::storage::groups::create_group;
        use crate::storage::sensors::create_sensor;
        use crate::storage::status::record_poll;
        use crate::web::testutil::*;

        let h = harness(true).await;
        let pool = &h.state.pool;
        let network = create_group(pool, "Netzwerk", None).await.unwrap();
        let routers = create_group(pool, "Router", Some(network)).await.unwrap();
        let office = create_group(pool, "Büro", None).await.unwrap();
        let now = crate::web::sensors::now_ms();
        let mut ping = HashMap::new();
        for (name, group) in [
            ("Switch", network),
            ("Gateway", routers),
            ("Drucker", office),
        ] {
            let device = devices::save_device(
                pool,
                None,
                DeviceInput {
                    kind: Default::default(),
                    name: name.to_owned(),
                    host: "192.0.2.1".to_owned(),
                    group_id: Some(group),
                    profile_id: None,
                    polling: Polling::default(),
                },
            )
            .await
            .unwrap();
            let sensor = create_sensor(pool, device, "Ping", &SensorSpec::Ping)
                .await
                .unwrap();
            ping.insert(name, (device, sensor));
        }
        for name in ["Switch", "Drucker"] {
            let assessment = Assessment::Healthy("OK".into());
            record_poll(pool, ping[name].1, now, &[], &assessment)
                .await
                .unwrap();
        }
        for ts in [now - 120_000, now - 60_000] {
            let assessment = Assessment::Failed("weg".into());
            record_poll(pool, ping["Gateway"].1, ts, &[], &assessment)
                .await
                .unwrap();
        }
        let (viewer, _) = login(&h, "bert").await;

        // The overview links the group names to their pages.
        let home = send(&h.app, get("/", Some(&viewer))).await;
        assert!(home.body.contains(&format!(
            "<a class=\"group-name\" href=\"/groups/{routers}\">"
        )));

        let page = send(&h.app, get(&format!("/groups/{network}"), Some(&viewer))).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains("<h1>Netzwerk</h1>"));
        assert!(page.body.contains(
            "<div class=\"breadcrumb\"><a href=\"/\">Übersicht</a><span>›</span><span>Netzwerk</span>"
        ));
        assert!(page.body.contains("Switch") && page.body.contains("Gateway"));
        assert!(!page.body.contains("Drucker"), "other groups stay out");
        assert!(page.body.contains("2 Geräte") && page.body.contains("1 Gerät mit Ausfall"));
        assert!(page.body.contains("<span class=\"badge status-down\">"));
        // Tiles count the subtree and open its status lists.
        assert!(page.body.contains(&format!(
            "<a class=\"tile status-down active\" href=\"/status/down?group={network}\"><div class=\"tile-value\">1</div>"
        )));
        assert!(page.body.contains(&format!(
            "href=\"/status/all?group={network}\"><div class=\"tile-value\">2</div>"
        )));
        assert!(
            !page.body.contains("/edit?back="),
            "no admin actions for viewers"
        );

        let sub = send(&h.app, get(&format!("/groups/{routers}"), Some(&viewer))).await;
        assert!(sub.body.contains(&format!(
            "<a href=\"/groups/{network}\">Netzwerk</a><span>›</span><span>Router</span>"
        )));
        assert!(!sub.body.contains("Switch"));
        assert_eq!(
            send(&h.app, get("/groups/999", Some(&viewer))).await.status,
            StatusCode::NOT_FOUND
        );

        // Status lists limited to a group and its subgroups.
        let list = send(
            &h.app,
            get(&format!("/status/all?group={network}"), Some(&viewer)),
        )
        .await;
        for (name, shown) in [("Switch", true), ("Gateway", true), ("Drucker", false)] {
            let link = format!("href=\"/sensors/{}\"", ping[name].1);
            assert_eq!(list.body.contains(&link), shown, "{name}");
        }
        assert!(list.body.contains("in „Netzwerk“ mit Untergruppen"));
        assert!(
            list.body
                .contains("<a href=\"/status/all\">alle Gruppen</a>")
        );
        assert!(list.body.contains(&format!(
            "href=\"/status/all?group={network}&#38;sort=sensor"
        )));
        assert!(list.body.contains(&format!(
            "<a href=\"/groups/{network}\">Netzwerk</a><span>›</span><span>Alle Sensoren</span>"
        )));
        assert_eq!(
            send(&h.app, get("/status/all?group=999", Some(&viewer)))
                .await
                .status,
            StatusCode::NOT_FOUND
        );

        // Admins get the actions; their pages lead back to the group page.
        let (admin, csrf) = login(&h, "anna").await;
        let back = format!("back=group-{network}");
        let page = send(&h.app, get(&format!("/groups/{network}"), Some(&admin))).await;
        for link in [
            format!("/groups/{network}/edit?{back}"),
            format!("/pause/group/{network}?{back}"),
            format!("/groups/{network}/order?{back}"),
            format!("/groups/new?parent={network}&{back}"),
            format!("/devices/new?group={network}&{back}"),
            format!("/pause/device/{}?{back}", ping["Gateway"].0),
            format!("/groups/{routers}/edit?{back}"),
        ] {
            assert!(page.body.contains(&format!("href=\"{link}\"")), "{link}");
        }
        let reply = send(
            &h.app,
            post(
                &format!("/pause/group/{network}"),
                Some(&admin),
                &format!("csrf_token={csrf}&duration=0&{back}"),
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/groups/{network}?msg=group_paused")
        );
        let page = send(
            &h.app,
            get(&format!("/groups/{network}?msg=group_paused"), Some(&admin)),
        )
        .await;
        assert!(page.body.contains("Gruppe pausiert."), "{}", page.body);
        assert!(page.body.contains("keins ihrer Geräte abgefragt"));
        assert!(page.body.contains("<span class=\"badge status-paused\">"));
        let reply = send(
            &h.app,
            post(
                &format!("/resume/group/{network}"),
                Some(&admin),
                &format!("csrf_token={csrf}&{back}"),
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/groups/{network}?msg=group_resumed")
        );

        let new_group = send(
            &h.app,
            get(
                &format!("/groups/new?parent={network}&{back}"),
                Some(&admin),
            ),
        )
        .await;
        assert!(
            new_group
                .body
                .contains(&format!("<option value=\"{network}\" selected>"))
        );
        assert!(
            new_group
                .body
                .contains(&format!("href=\"/groups/{network}\">Abbrechen</a>"))
        );
        let reply = send(
            &h.app,
            post(
                "/groups",
                Some(&admin),
                &format!("csrf_token={csrf}&name=Keller&parent_id={network}&{back}"),
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/groups/{network}?msg=group_created")
        );
        let reply = send(
            &h.app,
            post(
                &format!("/groups/{network}"),
                Some(&admin),
                &format!("csrf_token={csrf}&name=Netz&parent_id=&{back}"),
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/groups/{network}?msg=group_updated")
        );

        let order = send(
            &h.app,
            get(&format!("/groups/{network}/order?{back}"), Some(&admin)),
        )
        .await;
        assert!(
            order
                .body
                .contains(&format!("href=\"/groups/{network}\">Abbrechen</a>"))
        );
        assert!(
            order
                .body
                .contains(&format!("<a href=\"/groups/{network}\">Netz</a>"))
        );
        let reply = send(
            &h.app,
            post(
                &format!("/groups/{network}/order"),
                Some(&admin),
                &format!("csrf_token={csrf}&action=reset&{back}"),
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/groups/{network}?msg=order_reset")
        );
    }
}
