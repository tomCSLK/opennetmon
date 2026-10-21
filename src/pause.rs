//! Pausing like PRTG: groups, devices and sensors can be paused for a while or until resumed, with an
//! optional reason. Everything below a paused object is paused too: it is not polled and sends no
//! notifications. Storage: `storage::pauses`.

use std::collections::HashMap;

use crate::i18n::Msg;

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
        }
    }
}

/// All pauses, to tell for any object whether and why it is paused (own pause first, then its
/// device, then its groups from the nearest up).
#[derive(Debug, Default)]
pub struct Pauses {
    pub(crate) now_ms: i64,
    /// Group → (name, parent, own pause).
    pub(crate) groups: HashMap<i64, (String, Option<i64>, Option<Pause>)>,
    /// Device → (group, own pause).
    pub(crate) devices: HashMap<i64, (Option<i64>, Option<Pause>)>,
    /// Sensors paused by themselves.
    pub(crate) sensors: HashMap<i64, Pause>,
}

impl Pauses {
    fn active(&self, pause: Option<&Pause>) -> Option<Pause> {
        pause.filter(|p| p.active_at(self.now_ms)).cloned()
    }

    /// The pause of a group: its own, else the nearest paused group above it.
    pub fn group(&self, id: i64) -> Option<Why> {
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

    /// The pause of a device: its own, else one of its groups.
    pub fn device(&self, id: i64) -> Option<Why> {
        let (group, pause) = self.devices.get(&id)?;
        if let Some(pause) = self.active(pause.as_ref()) {
            return Some(Why {
                source: Source::Own,
                pause,
            });
        }
        let why = self.group((*group)?)?;
        Some(Why {
            source: match why.source {
                Source::Own => Source::Group(self.groups.get(&(*group)?)?.0.clone()),
                other => other,
            },
            pause: why.pause,
        })
    }

    /// The pause of a sensor: its own, else its device's (or a group's).
    pub fn sensor(&self, id: i64, device_id: i64) -> Option<Why> {
        if let Some(pause) = self.active(self.sensors.get(&id)) {
            return Some(Why {
                source: Source::Own,
                pause,
            });
        }
        let why = self.device(device_id)?;
        Some(Why {
            source: match why.source {
                Source::Own => Source::Device,
                other => other,
            },
            pause: why.pause,
        })
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
