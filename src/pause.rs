//! Pausing like PRTG: groups, devices and sensors can be paused for a while or until resumed, with an
//! optional reason. Everything below a paused object is paused too: it is not polled and sends no
//! notifications. Storage: `storage::pauses`.
//!
//! Dependencies work like pauses that come and go by themselves: an object that depends on a sensor
//! is paused while that sensor is down or paused (by hand, with its device or group, or through its
//! own dependency, so chains like router → switch → access point work), and for a delay after it is
//! fine again. The setting is inherited from group to subgroup, device and sensor like a pause; a
//! master sensor pauses the other sensors of its device. The sensor depended on is always polled.
//!
//! Maintenance windows ([`crate::schedule`]) pause the objects their schedule applies to (inherited
//! like a dependency) while a window is open; a manual pause comes first.

use std::collections::{HashMap, HashSet};

use crate::i18n::Msg;
use crate::schedule::Assignment;
use crate::status::Status;

pub const MAX_REASON_LEN: usize = 200;
/// The durations the form offers, in minutes; 0 = until resumed.
pub const DURATIONS_MIN: [u32; 6] = [0, 15, 60, 180, 1440, 10_080];

/// What is paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    Group(i64),
    Device(i64),
    Sensor(i64),
}

impl Target {
    /// From the path of the pause pages (`/pause/<kind>/<id>`).
    pub fn parse(kind: &str, id: i64) -> Option<Self> {
        match kind {
            "group" => Some(Self::Group(id)),
            "device" => Some(Self::Device(id)),
            "sensor" => Some(Self::Sensor(id)),
            _ => None,
        }
    }

    pub fn kind(self) -> &'static str {
        match self {
            Self::Group(_) => "group",
            Self::Device(_) => "device",
            Self::Sensor(_) => "sensor",
        }
    }

    pub fn id(self) -> i64 {
        match self {
            Self::Group(id) | Self::Device(id) | Self::Sensor(id) => id,
        }
    }
}

/// The pause of one object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pause {
    /// `None` = until resumed.
    pub until_ms: Option<i64>,
    pub reason: String,
}

impl Pause {
    /// A timed pause is over once its end has passed, even before `storage::pauses::expire` ran.
    pub fn active_at(&self, now_ms: i64) -> bool {
        self.until_ms.is_none_or(|until| until > now_ms)
    }
}

/// Where the pause of an object comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The object itself.
    Own,
    /// Its device (for a sensor).
    Device,
    /// A group above it (or the group itself, for a group: then `Own`).
    Group(String),
    /// A sensor it depends on is down or paused (only for sensors).
    Dependency {
        sensor_id: i64,
        sensor: String,
        device: String,
    },
    /// A window of a schedule that applies to it is open.
    Schedule { id: i64, name: String },
}

/// Why an object is paused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Why {
    pub source: Source,
    pub pause: Pause,
}

impl Why {
    /// Text for status messages, e.g. "Pausiert mit der Gruppe „Netzwerk“" (the end and the reason
    /// are shown separately, the end as a local time).
    pub fn label(&self) -> Msg {
        match &self.source {
            Source::Own => Msg::new("pause-note-own"),
            Source::Device => Msg::new("pause-note-device"),
            Source::Group(name) => Msg::new("pause-note-group").with("name", name),
            Source::Dependency { sensor, device, .. } => Msg::new("pause-note-dependency")
                .with("sensor", sensor)
                .with("device", device),
            Source::Schedule { name, .. } => Msg::new("pause-note-schedule").with("name", name),
        }
    }

    /// The sensor whose state pauses the object, if it is paused through a dependency.
    pub fn dependency(&self) -> Option<i64> {
        match self.source {
            Source::Dependency { sensor_id, .. } => Some(sensor_id),
            _ => None,
        }
    }

    /// The schedule whose window pauses the object.
    pub fn schedule(&self) -> Option<i64> {
        match self.source {
            Source::Schedule { id, .. } => Some(id),
            _ => None,
        }
    }
}

/// A schedule as pauses see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleInfo {
    pub name: String,
    /// The end of the window open now, if one is.
    pub open_until: Option<i64>,
}

/// The dependency setting of a group, device or sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dependency {
    /// Like the object above (for a group without parent: none).
    #[default]
    Parent,
    /// Paused while this sensor is down or paused.
    Sensor(i64),
    /// No dependency, whatever the objects above say.
    None,
}

impl Dependency {
    /// From the columns `dependency` and `dependency_sensor_id`; a deleted sensor reads as `Parent`.
    pub fn from_columns(kind: &str, sensor: Option<i64>) -> Self {
        match (kind, sensor) {
            ("sensor", Some(id)) => Self::Sensor(id),
            ("none", _) => Self::None,
            _ => Self::Parent,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Parent => "parent",
            Self::Sensor(_) => "sensor",
            Self::None => "none",
        }
    }

    pub fn sensor_id(self) -> Option<i64> {
        match self {
            Self::Sensor(id) => Some(id),
            _ => None,
        }
    }
}

/// Longest delay after the sensor depended on is fine again, in seconds (like the longest interval).
pub const MAX_DELAY_SECS: u32 = 3_600;

/// The dependency settings of one object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Setting {
    pub dependency: Dependency,
    /// How long dependent objects stay paused after the sensor is fine again; `None` = like the
    /// object above (in the end 0).
    pub delay_secs: Option<u32>,
}

/// The delay field of the form: empty = like the object above, otherwise 0 to [`MAX_DELAY_SECS`].
pub fn parse_delay(text: &str) -> Result<Option<u32>, Msg> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<u32>() {
        Ok(secs) if secs <= MAX_DELAY_SECS => Ok(Some(secs)),
        _ => Err(Msg::new("dependency-delay-invalid").with("max", MAX_DELAY_SECS)),
    }
}

/// What the poller last stored for a sensor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    pub status: Status,
    /// Failed polls in a row.
    pub failures: u32,
    /// The last status change: when, and the status before.
    pub last_change: Option<(i64, Option<Status>)>,
}

/// A sensor as dependencies see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SensorInfo {
    pub device_id: i64,
    pub name: String,
    pub device: String,
    pub setting: Setting,
    pub master: bool,
    /// `None` before its first poll.
    pub observed: Option<Observed>,
}

/// Where an effective setting comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Own,
    Device,
    Group(String),
    /// Nothing is set anywhere above.
    Default,
}

/// The dependency that applies to an object and where it comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effective {
    /// The sensor depended on; `None` = no dependency.
    pub sensor: Option<i64>,
    pub from: Origin,
    pub delay_secs: u32,
    pub delay_from: Origin,
}

/// How much a sensor pauses through dependencies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Affected {
    pub sensors: usize,
    pub devices: HashSet<i64>,
}

/// All pauses, to tell for any object whether and why it is paused (own pause first, then its
/// device, then its groups from the nearest up, then its dependencies).
#[derive(Debug, Default, Clone)]
pub struct Pauses {
    pub(crate) now_ms: i64,
    /// Group → (name, parent, own pause).
    pub(crate) groups: HashMap<i64, (String, Option<i64>, Option<Pause>)>,
    /// Device → (group, own pause).
    pub(crate) devices: HashMap<i64, (Option<i64>, Option<Pause>)>,
    /// Sensors paused by themselves.
    pub(crate) sensors: HashMap<i64, Pause>,
    /// Dependency settings of groups and devices (missing = the default).
    pub(crate) group_settings: HashMap<i64, Setting>,
    pub(crate) device_settings: HashMap<i64, Setting>,
    /// Every sensor, with its dependency settings and last state.
    pub(crate) sensor_info: HashMap<i64, SensorInfo>,
    /// Every schedule, and which applies to which object (missing = like the object above).
    pub(crate) schedules: HashMap<i64, ScheduleInfo>,
    pub(crate) group_schedules: HashMap<i64, Assignment>,
    pub(crate) device_schedules: HashMap<i64, Assignment>,
    pub(crate) sensor_schedules: HashMap<i64, Assignment>,
}

impl Pauses {
    fn active(&self, pause: Option<&Pause>) -> Option<Pause> {
        pause.filter(|p| p.active_at(self.now_ms)).cloned()
    }

    /// The pause of a group: its own, else the nearest paused group above it, else an open window
    /// of its schedule.
    pub fn group(&self, id: i64) -> Option<Why> {
        self.group_by_hand(id)
            .or_else(|| self.maintenance(Target::Group(id)))
    }

    /// A pause by hand of the group or a group above it.
    fn group_by_hand(&self, id: i64) -> Option<Why> {
        let mut current = Some(id);
        // At most as many steps as there are groups, so corrupted cyclic data terminates.
        for _ in 0..=self.groups.len() {
            let (name, parent, pause) = self.groups.get(&current?)?;
            if let Some(pause) = self.active(pause.as_ref()) {
                let source = if current == Some(id) {
                    Source::Own
                } else {
                    Source::Group(name.clone())
                };
                return Some(Why { source, pause });
            }
            current = *parent;
        }
        None
    }

    /// The pause of a device: its own, else one of its groups, else an open window of its schedule.
    pub fn device(&self, id: i64) -> Option<Why> {
        self.device_by_hand(id)
            .or_else(|| self.maintenance(Target::Device(id)))
    }

    /// A pause by hand of the device or one of its groups.
    fn device_by_hand(&self, id: i64) -> Option<Why> {
        let (group, pause) = self.devices.get(&id)?;
        if let Some(pause) = self.active(pause.as_ref()) {
            return Some(Why {
                source: Source::Own,
                pause,
            });
        }
        let why = self.group_by_hand((*group)?)?;
        Some(Why {
            source: match why.source {
                Source::Own => Source::Group(self.groups.get(&(*group)?)?.0.clone()),
                other => other,
            },
            pause: why.pause,
        })
    }

    /// The pause of a sensor: its own, else its device's (or a group's), else an open window of its
    /// schedule, else one through a dependency.
    pub fn sensor(&self, id: i64, device_id: i64) -> Option<Why> {
        self.sensor_on_path(id, device_id, &mut Vec::new())
    }

    /// `path`: the sensors whose pause is being decided, to break cyclic dependencies.
    fn sensor_on_path(&self, id: i64, device_id: i64, path: &mut Vec<i64>) -> Option<Why> {
        if let Some(pause) = self.active(self.sensors.get(&id)) {
            return Some(Why {
                source: Source::Own,
                pause,
            });
        }
        if let Some(why) = self.device_by_hand(device_id) {
            return Some(Why {
                source: match why.source {
                    Source::Own => Source::Device,
                    other => other,
                },
                pause: why.pause,
            });
        }
        if let Some(why) = self.maintenance(Target::Sensor(id)) {
            return Some(why);
        }
        if path.contains(&id) {
            return None;
        }
        path.push(id);
        let delay = self.effective(Target::Sensor(id)).delay_secs;
        let why = self.targets(id, device_id).into_iter().find_map(|target| {
            let until_ms = self.triggered(target, delay, path)?;
            let info = self.sensor_info.get(&target)?;
            Some(Why {
                source: Source::Dependency {
                    sensor_id: target,
                    sensor: info.name.clone(),
                    device: info.device.clone(),
                },
                pause: Pause {
                    until_ms,
                    reason: String::new(),
                },
            })
        });
        path.pop();
        why
    }

    /// The sensors a sensor depends on: the one of its setting (own or inherited) and the master
    /// sensor of its device, never itself.
    fn targets(&self, id: i64, device_id: i64) -> Vec<i64> {
        let mut targets: Vec<i64> = [
            self.effective(Target::Sensor(id)).sensor,
            self.master_of(device_id),
        ]
        .into_iter()
        .flatten()
        .filter(|t| *t != id)
        .collect();
        targets.dedup();
        targets
    }

    /// Whether `target` pauses the sensors that depend on it: `Some(end)` if so (`None` = while it
    /// lasts, else the end of the delay after it was fine again).
    fn triggered(&self, target: i64, delay_secs: u32, path: &mut Vec<i64>) -> Option<Option<i64>> {
        let info = self.sensor_info.get(&target)?;
        let observed = info.observed.as_ref();
        let stopped = |s: Status| matches!(s, Status::Down | Status::Paused);
        if observed.is_some_and(|o| stopped(o.status))
            || self.sensor_on_path(target, info.device_id, path).is_some()
        {
            return Some(None);
        }
        let (ts, from) = observed?.last_change?;
        let until = ts.saturating_add(i64::from(delay_secs) * 1_000);
        (delay_secs > 0 && from.is_some_and(stopped) && until > self.now_ms).then_some(Some(until))
    }

    /// Whether a failed poll of the sensor must not count yet: a sensor it depends on has failed
    /// polls of its own (or none yet) and may be about to pause it, as when the router stops
    /// answering just before the switch behind it.
    pub fn dependency_unsettled(&self, id: i64, device_id: i64) -> bool {
        self.targets(id, device_id).into_iter().any(|target| {
            self.sensor_info
                .get(&target)
                .and_then(|info| info.observed.as_ref())
                .is_none_or(|o| o.failures > 0)
        })
    }

    /// The objects an object's setting is inherited from, nearest first, starting with itself.
    fn chain(&self, target: Target) -> Vec<(Origin, Setting)> {
        let mut chain = Vec::new();
        let mut group = match target {
            Target::Sensor(id) => {
                let Some(info) = self.sensor_info.get(&id) else {
                    return chain;
                };
                chain.push((Origin::Own, info.setting));
                chain.push((
                    Origin::Device,
                    self.device_settings
                        .get(&info.device_id)
                        .copied()
                        .unwrap_or_default(),
                ));
                self.devices.get(&info.device_id).and_then(|(g, _)| *g)
            }
            Target::Device(id) => {
                chain.push((
                    Origin::Own,
                    self.device_settings.get(&id).copied().unwrap_or_default(),
                ));
                self.devices.get(&id).and_then(|(g, _)| *g)
            }
            Target::Group(id) => Some(id),
        };
        for _ in 0..=self.groups.len() {
            let Some(id) = group else { break };
            let Some((name, parent, _)) = self.groups.get(&id) else {
                break;
            };
            let origin = if target == Target::Group(id) {
                Origin::Own
            } else {
                Origin::Group(name.clone())
            };
            chain.push((
                origin,
                self.group_settings.get(&id).copied().unwrap_or_default(),
            ));
            group = *parent;
        }
        chain
    }

    /// The dependency and delay that apply to an object (its own setting or the nearest above).
    pub fn effective(&self, target: Target) -> Effective {
        let chain = self.chain(target);
        let (sensor, from) = chain
            .iter()
            .find(|(_, s)| s.dependency != Dependency::Parent)
            .map_or((None, Origin::Default), |(origin, s)| {
                (s.dependency.sensor_id(), origin.clone())
            });
        let (delay_secs, delay_from) = chain
            .iter()
            .find_map(|(origin, s)| s.delay_secs.map(|d| (d, origin.clone())))
            .unwrap_or((0, Origin::Default));
        Effective {
            sensor,
            from,
            delay_secs,
            delay_from,
        }
    }

    /// The schedule assignments an object inherits from, nearest first, starting with itself.
    fn schedule_chain(&self, target: Target) -> Vec<(Origin, Assignment)> {
        let mut chain = Vec::new();
        let mut group = match target {
            Target::Sensor(id) => {
                chain.push((
                    Origin::Own,
                    self.sensor_schedules.get(&id).copied().unwrap_or_default(),
                ));
                let Some(device) = self.sensor_info.get(&id).map(|i| i.device_id) else {
                    return chain;
                };
                chain.push((
                    Origin::Device,
                    self.device_schedules
                        .get(&device)
                        .copied()
                        .unwrap_or_default(),
                ));
                self.devices.get(&device).and_then(|(g, _)| *g)
            }
            Target::Device(id) => {
                chain.push((
                    Origin::Own,
                    self.device_schedules.get(&id).copied().unwrap_or_default(),
                ));
                self.devices.get(&id).and_then(|(g, _)| *g)
            }
            Target::Group(id) => Some(id),
        };
        for _ in 0..=self.groups.len() {
            let Some(id) = group else { break };
            let Some((name, parent, _)) = self.groups.get(&id) else {
                break;
            };
            let origin = if target == Target::Group(id) {
                Origin::Own
            } else {
                Origin::Group(name.clone())
            };
            chain.push((
                origin,
                self.group_schedules.get(&id).copied().unwrap_or_default(),
            ));
            group = *parent;
        }
        chain
    }

    /// The schedule that applies to an object (its own or the nearest above) and where it comes from.
    pub fn schedule_of(&self, target: Target) -> (Option<i64>, Origin) {
        self.schedule_chain(target)
            .into_iter()
            .find(|(_, a)| *a != Assignment::Parent)
            .map_or((None, Origin::Default), |(origin, a)| {
                (a.schedule_id(), origin)
            })
    }

    /// What an object would inherit with no schedule setting of its own.
    pub fn schedule_inherited(&self, target: Target) -> (Option<i64>, Origin) {
        self.schedule_chain(target)
            .into_iter()
            .skip(1)
            .find(|(_, a)| *a != Assignment::Parent)
            .map_or((None, Origin::Default), |(origin, a)| {
                (a.schedule_id(), origin)
            })
    }

    /// The own schedule setting of an object.
    pub fn schedule_setting(&self, target: Target) -> Assignment {
        match target {
            Target::Group(id) => self.group_schedules.get(&id),
            Target::Device(id) => self.device_schedules.get(&id),
            Target::Sensor(id) => self.sensor_schedules.get(&id),
        }
        .copied()
        .unwrap_or_default()
    }

    pub fn schedule_info(&self, id: i64) -> Option<&ScheduleInfo> {
        self.schedules.get(&id)
    }

    /// The pause through an open window of the object's schedule.
    fn maintenance(&self, target: Target) -> Option<Why> {
        let id = self.schedule_of(target).0?;
        let info = self.schedules.get(&id)?;
        let until = info.open_until.filter(|until| *until > self.now_ms)?;
        Some(Why {
            source: Source::Schedule {
                id,
                name: info.name.clone(),
            },
            pause: Pause {
                until_ms: Some(until),
                reason: String::new(),
            },
        })
    }

    /// Per schedule with an open window: how many sensors it pauses and on how many devices.
    pub fn schedule_roots(&self) -> HashMap<i64, Affected> {
        let mut roots: HashMap<i64, Affected> = HashMap::new();
        for (id, info) in &self.sensor_info {
            if let Some(schedule) = self.sensor(*id, info.device_id).and_then(|w| w.schedule()) {
                let affected = roots.entry(schedule).or_default();
                affected.sensors += 1;
                affected.devices.insert(info.device_id);
            }
        }
        roots
    }

    /// The own dependency setting of an object.
    pub fn setting(&self, target: Target) -> Setting {
        match target {
            Target::Group(id) => self.group_settings.get(&id).copied(),
            Target::Device(id) => self.device_settings.get(&id).copied(),
            Target::Sensor(id) => self.sensor_info.get(&id).map(|i| i.setting),
        }
        .unwrap_or_default()
    }

    /// What an object would inherit with no setting of its own.
    pub fn inherited(&self, target: Target) -> Effective {
        let master = match target {
            Target::Sensor(id) => self.sensor_info.get(&id).map(|i| i.master),
            _ => None,
        };
        self.with_setting(target, Setting::default(), master)
            .effective(target)
    }

    /// The master sensor of a device, if it has one.
    pub fn master_of(&self, device_id: i64) -> Option<i64> {
        self.sensor_info
            .iter()
            .find(|(_, info)| info.master && info.device_id == device_id)
            .map(|(id, _)| *id)
    }

    /// The sensor and its device by name, e.g. for a page that names what is depended on.
    pub fn sensor_info(&self, id: i64) -> Option<&SensorInfo> {
        self.sensor_info.get(&id)
    }

    /// The sensors that pause others through dependencies, each with what it pauses (also through
    /// chains: what a paused switch pauses counts for the router that is down).
    pub fn dependency_roots(&self) -> HashMap<i64, Affected> {
        let mut roots: HashMap<i64, Affected> = HashMap::new();
        for (id, info) in &self.sensor_info {
            let Some(mut root) = self
                .sensor(*id, info.device_id)
                .and_then(|w| w.dependency())
            else {
                continue;
            };
            for _ in 0..self.sensor_info.len() {
                let next = self
                    .sensor_info
                    .get(&root)
                    .and_then(|r| self.sensor(root, r.device_id))
                    .and_then(|w| w.dependency());
                match next {
                    Some(next) if next != root => root = next,
                    _ => break,
                }
            }
            let affected = roots.entry(root).or_default();
            affected.sensors += 1;
            affected.devices.insert(info.device_id);
        }
        roots
    }

    /// Whether some sensor would, through dependencies, depend on itself.
    pub fn has_cycle(&self) -> bool {
        // Depth-first search; 1 = on the current path, 2 = done.
        let mut marks: HashMap<i64, u8> = HashMap::new();
        let mut ids: Vec<i64> = self.sensor_info.keys().copied().collect();
        ids.sort_unstable();
        for start in ids {
            if marks.contains_key(&start) {
                continue;
            }
            let mut stack = vec![(start, false)];
            while let Some((id, leaving)) = stack.pop() {
                if leaving {
                    marks.insert(id, 2);
                    continue;
                }
                match marks.get(&id) {
                    Some(2) => continue,
                    Some(_) => return true,
                    None => {}
                }
                marks.insert(id, 1);
                stack.push((id, true));
                let device = self.sensor_info.get(&id).map_or(0, |i| i.device_id);
                for target in self.targets(id, device) {
                    match marks.get(&target) {
                        Some(1) => return true,
                        Some(_) => {}
                        None => stack.push((target, false)),
                    }
                }
            }
        }
        false
    }

    /// The same pauses with another dependency setting for one object (to check it before saving);
    /// `master` changes the master flag of a sensor (and clears the others of its device).
    pub fn with_setting(&self, target: Target, setting: Setting, master: Option<bool>) -> Self {
        let mut next = self.clone();
        match target {
            Target::Group(id) => {
                next.group_settings.insert(id, setting);
            }
            Target::Device(id) => {
                next.device_settings.insert(id, setting);
            }
            Target::Sensor(id) => {
                let device = next.sensor_info.get(&id).map(|i| i.device_id);
                if master == Some(true) {
                    for info in next.sensor_info.values_mut() {
                        if Some(info.device_id) == device {
                            info.master = false;
                        }
                    }
                }
                if let Some(info) = next.sensor_info.get_mut(&id) {
                    info.setting = setting;
                    if let Some(master) = master {
                        info.master = master;
                    }
                }
            }
        }
        next
    }
}

/// The duration field of the form: minutes from [`DURATIONS_MIN`]; `None` = until resumed.
pub fn parse_duration(text: &str) -> Result<Option<u32>, Msg> {
    let minutes: u32 = text
        .trim()
        .parse()
        .map_err(|_| Msg::new("pause-duration-invalid"))?;
    if !DURATIONS_MIN.contains(&minutes) {
        return Err(Msg::new("pause-duration-invalid"));
    }
    Ok((minutes > 0).then_some(minutes))
}

/// Trims the reason; at most [`MAX_REASON_LEN`] characters, no control characters.
pub fn validate_reason(text: &str) -> Result<String, Msg> {
    let text = text.trim();
    if text.chars().count() > MAX_REASON_LEN || text.chars().any(char::is_control) {
        return Err(Msg::new("pause-reason-invalid"));
    }
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pause(until_ms: Option<i64>, reason: &str) -> Option<Pause> {
        Some(Pause {
            until_ms,
            reason: reason.to_owned(),
        })
    }

    /// Netzwerk (1) › Switche (2) › Rack (3); device 10 in Rack, device 11 in Switche.
    fn tree() -> Pauses {
        let mut p = Pauses {
            now_ms: 1_000,
            ..Pauses::default()
        };
        p.groups.insert(1, ("Netzwerk".into(), None, None));
        p.groups.insert(2, ("Switche".into(), Some(1), None));
        p.groups.insert(3, ("Rack".into(), Some(2), None));
        p.devices.insert(10, (Some(3), None));
        p.devices.insert(11, (Some(2), None));
        p.devices.insert(12, (None, None));
        p
    }

    #[test]
    fn nothing_is_paused_without_pauses() {
        let p = tree();
        assert_eq!(p.sensor(100, 10), None);
        assert_eq!(p.device(12), None);
        assert_eq!(p.group(3), None);
        assert_eq!(p.sensor(100, 99), None, "unknown device");
    }

    #[test]
    fn a_paused_group_pauses_everything_below_it() {
        let mut p = tree();
        p.groups.get_mut(&2).unwrap().2 = pause(None, "Umbau");
        assert_eq!(p.group(2).unwrap().source, Source::Own);
        assert_eq!(p.group(3).unwrap().source, Source::Group("Switche".into()));
        assert_eq!(p.group(1), None, "not upwards");
        let why = p.sensor(100, 10).unwrap();
        assert_eq!(why.source, Source::Group("Switche".into()));
        assert_eq!(why.pause.reason, "Umbau");
        assert_eq!(
            p.device(11).unwrap().source,
            Source::Group("Switche".into()),
            "the device's own group is named too"
        );
        assert_eq!(p.device(12), None);
    }

    #[test]
    fn the_nearest_pause_wins_and_ended_ones_do_not_count() {
        let mut p = tree();
        p.groups.get_mut(&1).unwrap().2 = pause(None, "oben");
        p.devices.get_mut(&10).unwrap().1 = pause(Some(5_000), "Gerät");
        p.sensors.insert(
            100,
            Pause {
                until_ms: Some(1_000),
                reason: "vorbei".into(),
            },
        );
        let why = p.sensor(100, 10).unwrap();
        assert_eq!(
            (why.source, why.pause.reason.as_str()),
            (Source::Device, "Gerät")
        );
        p.now_ms = 5_000;
        let why = p.sensor(100, 10).unwrap();
        assert_eq!(
            (why.source, why.pause.reason.as_str()),
            (Source::Group("Netzwerk".into()), "oben")
        );
        p.sensors.insert(
            101,
            Pause {
                until_ms: None,
                reason: String::new(),
            },
        );
        assert_eq!(p.sensor(101, 12).unwrap().source, Source::Own);
    }

    #[test]
    fn cyclic_groups_terminate() {
        let mut p = tree();
        p.groups.get_mut(&1).unwrap().1 = Some(3);
        assert_eq!(p.sensor(100, 10), None);
    }

    fn observed(
        status: Status,
        failures: u32,
        last_change: Option<(i64, Option<Status>)>,
    ) -> Observed {
        Observed {
            status,
            failures,
            last_change,
        }
    }

    /// [`tree`] with the router (device 13 in Netzwerk, ping 130) and sensors 100 (device 10),
    /// 110 and 111 (device 11) and 120 (device 12), all up.
    fn with_sensors() -> Pauses {
        let mut p = tree();
        p.devices.insert(13, (Some(1), None));
        for (id, device, name, device_name) in [
            (130, 13, "Ping", "Router"),
            (100, 10, "Ping", "Rack-Switch"),
            (110, 11, "Ping", "Switch"),
            (111, 11, "Port 1", "Switch"),
            (120, 12, "Ping", "AP"),
        ] {
            p.sensor_info.insert(
                id,
                SensorInfo {
                    device_id: device,
                    name: name.into(),
                    device: device_name.into(),
                    setting: Setting::default(),
                    master: false,
                    observed: Some(observed(Status::Up, 0, None)),
                },
            );
        }
        p
    }

    #[test]
    fn an_open_window_pauses_what_its_schedule_applies_to() {
        let mut p = with_sensors();
        p.schedules.insert(
            7,
            ScheduleInfo {
                name: "Nachts".into(),
                open_until: Some(5_000),
            },
        );
        p.schedules.insert(
            8,
            ScheduleInfo {
                name: "Nie offen".into(),
                open_until: None,
            },
        );
        p.group_schedules.insert(2, Assignment::Schedule(7));
        let night = Source::Schedule {
            id: 7,
            name: "Nachts".into(),
        };
        let why = p.sensor(100, 10).unwrap();
        assert_eq!(why.source, night, "inherited from Switche through Rack");
        assert_eq!(why.pause.until_ms, Some(5_000));
        assert_eq!(p.group(3).unwrap().source, night);
        assert_eq!(p.device(11).unwrap().source, night);
        assert_eq!(p.group(1), None, "not upwards");
        assert_eq!(p.sensor(130, 13), None);
        assert_eq!(
            p.schedule_of(Target::Sensor(110)),
            (Some(7), Origin::Group("Switche".into()))
        );

        // "None" ends the inheritance; a closed schedule pauses nothing.
        p.device_schedules.insert(11, Assignment::None);
        assert_eq!(p.sensor(110, 11), None);
        assert_eq!(p.device(11), None);
        assert_eq!(p.schedule_inherited(Target::Device(11)).0, Some(7));
        p.sensor_schedules.insert(111, Assignment::Schedule(8));
        assert_eq!(p.sensor(111, 11), None);

        // A pause by hand comes first; dependents follow the sensor in its window.
        p.devices.get_mut(&10).unwrap().1 = pause(None, "Umbau");
        assert_eq!(p.sensor(100, 10).unwrap().source, Source::Device);
        p.sensor_info.get_mut(&120).unwrap().setting = depend(110);
        p.device_schedules.remove(&11);
        assert_eq!(paused_by(&p, 120), Some(110));
        let roots = p.schedule_roots();
        assert_eq!(
            roots[&7].sensors, 1,
            "only 110; 100 is paused by hand, 111 has its own"
        );
        // The window closes.
        p.now_ms = 5_000;
        assert_eq!(p.sensor(110, 11), None);
        assert!(p.schedule_roots().is_empty());
    }

    fn depend(on: i64) -> Setting {
        Setting {
            dependency: Dependency::Sensor(on),
            delay_secs: None,
        }
    }

    fn set_state(p: &mut Pauses, id: i64, o: Observed) {
        p.sensor_info.get_mut(&id).unwrap().observed = Some(o);
    }

    fn paused_by(p: &Pauses, id: i64) -> Option<i64> {
        let device = p.sensor_info[&id].device_id;
        p.sensor(id, device).and_then(|w| w.dependency())
    }

    #[test]
    fn a_group_depending_on_the_router_ping_is_paused_while_it_is_down() {
        let mut p = with_sensors();
        p.group_settings.insert(2, depend(130));
        assert_eq!(paused_by(&p, 110), None, "the router is fine");
        set_state(&mut p, 130, observed(Status::Warning, 1, None));
        assert_eq!(paused_by(&p, 110), None, "a warning alone does not pause");
        set_state(&mut p, 130, observed(Status::Down, 2, None));
        let why = p.sensor(110, 11).unwrap();
        assert_eq!(
            why.source,
            Source::Dependency {
                sensor_id: 130,
                sensor: "Ping".into(),
                device: "Router".into()
            }
        );
        assert_eq!(why.pause.until_ms, None);
        assert_eq!(paused_by(&p, 100), Some(130), "inherited by the subgroup");
        assert_eq!(paused_by(&p, 120), None, "not in the group");
        assert_eq!(
            paused_by(&p, 130),
            None,
            "the router ping itself keeps running"
        );
        assert_eq!(p.device(11), None, "devices are not paused by themselves");

        // "No dependency" ends the inheritance.
        p.group_settings.insert(
            3,
            Setting {
                dependency: Dependency::None,
                delay_secs: None,
            },
        );
        assert_eq!(paused_by(&p, 100), None);
        assert_eq!(paused_by(&p, 110), Some(130));
    }

    #[test]
    fn a_sensor_never_depends_on_itself() {
        let mut p = with_sensors();
        // The router depends on its own ping: its other sensors would pause, the ping keeps running.
        p.device_settings.insert(13, depend(130));
        p.sensor_info.insert(
            131,
            SensorInfo {
                device_id: 13,
                name: "Uptime".into(),
                device: "Router".into(),
                setting: Setting::default(),
                master: false,
                observed: Some(observed(Status::Up, 0, None)),
            },
        );
        set_state(&mut p, 130, observed(Status::Down, 2, None));
        assert_eq!(paused_by(&p, 130), None);
        assert_eq!(paused_by(&p, 131), Some(130));
        assert!(!p.has_cycle());
    }

    #[test]
    fn chains_and_hand_pauses_pause_what_depends_on_them() {
        let mut p = with_sensors();
        p.group_settings.insert(2, depend(130));
        // The access point depends on the switch ping, which depends on the router.
        p.device_settings.insert(12, depend(110));
        set_state(&mut p, 130, observed(Status::Down, 2, None));
        assert_eq!(
            paused_by(&p, 120),
            Some(110),
            "the paused switch pauses the AP"
        );
        let roots = p.dependency_roots();
        assert_eq!(
            roots.len(),
            1,
            "everything goes back to the router: {roots:?}"
        );
        let router = &roots[&130];
        assert_eq!(router.sensors, 4, "100, 110, 111 and 120");
        assert_eq!(router.devices, HashSet::from([10, 11, 12]));

        // The router paused by hand (or with its device or group) pauses them too.
        set_state(&mut p, 130, observed(Status::Up, 0, None));
        assert_eq!(paused_by(&p, 110), None);
        p.groups.get_mut(&1).unwrap().2 = pause(None, "Wartung");
        assert_eq!(
            paused_by(&p, 110),
            None,
            "the switch is in the paused group itself"
        );
        assert_eq!(
            p.sensor(110, 11).unwrap().source,
            Source::Group("Netzwerk".into())
        );
        p.groups.get_mut(&1).unwrap().2 = None;
        p.sensors.insert(130, pause(None, "").unwrap());
        assert_eq!(paused_by(&p, 110), Some(130));
        assert_eq!(paused_by(&p, 120), Some(110));
        assert!(p.dependency_roots().contains_key(&130));
    }

    #[test]
    fn a_stored_paused_state_counts_until_the_next_poll() {
        let mut p = with_sensors();
        p.device_settings.insert(12, depend(110));
        // The switch ping was paused and has not been polled since it was resumed.
        set_state(&mut p, 110, observed(Status::Paused, 0, None));
        assert_eq!(paused_by(&p, 120), Some(110));
    }

    #[test]
    fn the_delay_keeps_them_paused_after_the_sensor_is_fine_again() {
        let mut p = with_sensors();
        p.group_settings.insert(
            2,
            Setting {
                dependency: Dependency::Sensor(130),
                delay_secs: Some(60),
            },
        );
        p.now_ms = 10_000;
        set_state(
            &mut p,
            130,
            observed(Status::Up, 0, Some((5_000, Some(Status::Down)))),
        );
        let why = p.sensor(110, 11).unwrap();
        assert_eq!(why.pause.until_ms, Some(65_000));
        p.now_ms = 65_000;
        assert_eq!(paused_by(&p, 110), None, "the delay is over");
        p.now_ms = 10_000;
        set_state(
            &mut p,
            130,
            observed(Status::Up, 0, Some((5_000, Some(Status::Warning)))),
        );
        assert_eq!(
            paused_by(&p, 110),
            None,
            "after a warning there is no delay"
        );

        // The delay is inherited on its own: the device sets it, the group the sensor.
        p.group_settings.insert(2, depend(130));
        p.device_settings.insert(
            11,
            Setting {
                dependency: Dependency::Parent,
                delay_secs: Some(30),
            },
        );
        let effective = p.effective(Target::Sensor(110));
        assert_eq!(
            effective,
            Effective {
                sensor: Some(130),
                from: Origin::Group("Switche".into()),
                delay_secs: 30,
                delay_from: Origin::Device,
            }
        );
        assert_eq!(
            p.effective(Target::Sensor(120)),
            Effective {
                sensor: None,
                from: Origin::Default,
                delay_secs: 0,
                delay_from: Origin::Default,
            }
        );
        assert_eq!(p.effective(Target::Group(2)).from, Origin::Own);
    }

    #[test]
    fn a_master_sensor_pauses_the_other_sensors_of_its_device() {
        let mut p = with_sensors();
        p.sensor_info.get_mut(&110).unwrap().master = true;
        set_state(&mut p, 110, observed(Status::Down, 2, None));
        assert_eq!(paused_by(&p, 111), Some(110));
        assert_eq!(paused_by(&p, 110), None, "the master itself keeps running");
        assert_eq!(paused_by(&p, 100), None, "other devices are not affected");
    }

    #[test]
    fn failures_do_not_count_while_the_sensor_depended_on_is_failing() {
        let mut p = with_sensors();
        p.group_settings.insert(2, depend(130));
        assert!(!p.dependency_unsettled(110, 11));
        set_state(&mut p, 130, observed(Status::Warning, 1, None));
        assert!(p.dependency_unsettled(110, 11));
        assert!(!p.dependency_unsettled(120, 12), "no dependency");
        // A warning from a limit (the ping answers, but slowly) is no reason.
        set_state(&mut p, 130, observed(Status::Warning, 0, None));
        assert!(!p.dependency_unsettled(110, 11));
        p.sensor_info.get_mut(&130).unwrap().observed = None;
        assert!(p.dependency_unsettled(110, 11), "not polled yet");
    }

    #[test]
    fn cycles_are_found_and_still_terminate() {
        let mut p = with_sensors();
        p.group_settings.insert(2, depend(120));
        assert!(!p.has_cycle());
        let cyclic = p.with_setting(Target::Device(12), depend(110), None);
        assert!(cyclic.has_cycle(), "110 → 120 → 110");
        assert!(!p.has_cycle(), "the original is unchanged");
        let mut cyclic = cyclic;
        set_state(&mut cyclic, 120, observed(Status::Up, 0, None));
        assert_eq!(paused_by(&cyclic, 110), None);
        // A master on the same device makes no cycle with itself.
        let master = p.with_setting(Target::Sensor(110), Setting::default(), Some(true));
        assert!(master.sensor_info[&110].master && !master.has_cycle());
        let moved = master.with_setting(Target::Sensor(111), Setting::default(), Some(true));
        assert!(
            !moved.sensor_info[&110].master && moved.sensor_info[&111].master,
            "one master per device"
        );
    }

    #[test]
    fn dependency_columns_and_the_delay_field_are_read() {
        assert_eq!(
            Dependency::from_columns("sensor", Some(4)),
            Dependency::Sensor(4)
        );
        assert_eq!(
            Dependency::from_columns("sensor", None),
            Dependency::Parent,
            "the sensor was deleted"
        );
        assert_eq!(Dependency::from_columns("none", None), Dependency::None);
        assert_eq!(Dependency::from_columns("x", None), Dependency::Parent);
        assert_eq!(parse_delay(" "), Ok(None));
        assert_eq!(parse_delay("120"), Ok(Some(120)));
        for bad in ["-1", "3601", "1.5", "x"] {
            assert!(parse_delay(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn form_values_are_checked() {
        assert_eq!(parse_duration("0"), Ok(None));
        assert_eq!(parse_duration(" 60 "), Ok(Some(60)));
        for bad in ["", "7", "-15", "x"] {
            assert!(parse_duration(bad).is_err(), "{bad}");
        }
        assert_eq!(validate_reason("  Wartung "), Ok("Wartung".to_owned()));
        assert!(validate_reason(&"x".repeat(MAX_REASON_LEN + 1)).is_err());
        assert!(validate_reason("a\nb").is_err());
        assert_eq!(
            Target::parse("device", 4).map(|t| (t.kind(), t.id())),
            Some(("device", 4))
        );
        assert_eq!(Target::parse("probe", 4), None);
    }
}
