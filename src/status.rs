//! Status logic: turns poll results into sensor states and derives device states.
//!
//! Pure functions only; persistence is in `storage::status`. Threshold evaluation per channel
//! (warning/down limits) plugs into [`assess`] later.

use crate::i18n::{Lang, Msg};
use crate::sensors::format::channel_label;
use crate::sensors::spec::SensorSpec;
use crate::sensors::{Measurement, SensorError};

/// A poll failure becomes `Down` only after this many consecutive failures; the first one is
/// a `Warning`. One lost UDP packet or a busy device must not raise a Down alarm.
pub const DOWN_AFTER_FAILURES: u32 = 2;

const MAX_MESSAGE_CHARS: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    Up,
    Warning,
    Down,
    Unknown,
    Paused,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Warning => "warning",
            Self::Down => "down",
            Self::Unknown => "unknown",
            Self::Paused => "paused",
        }
    }

    /// Catalog id of the UI label.
    pub fn label_id(self) -> &'static str {
        match self {
            Self::Up => "status-up",
            Self::Warning => "status-warning",
            Self::Down => "status-down",
            Self::Unknown => "status-unknown",
            Self::Paused => "status-paused",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "up" => Self::Up,
            "warning" => Self::Warning,
            "down" => Self::Down,
            "unknown" => Self::Unknown,
            "paused" => Self::Paused,
            _ => return None,
        })
    }

    /// Severity for aggregation: higher is worse. `Paused` does not take part.
    fn severity(self) -> u8 {
        match self {
            Self::Up | Self::Paused => 0,
            Self::Unknown => 1,
            Self::Warning => 2,
            Self::Down => 3,
        }
    }
}

/// Persisted state of one sensor.
#[derive(Debug, Clone, PartialEq)]
pub struct SensorState {
    pub status: Status,
    /// When the sensor entered `status`.
    pub since_ms: i64,
    pub last_poll_ms: i64,
    /// Stored encoded ([`Msg::encode`]), shown with [`shown`].
    pub message: Msg,
    pub consecutive_failures: u32,
}

/// Limits of one channel, in the channel's base unit. A value strictly beyond a limit trips it.
/// Down limits win over warning limits.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Thresholds {
    pub warn_above: Option<f64>,
    pub warn_below: Option<f64>,
    pub down_above: Option<f64>,
    pub down_below: Option<f64>,
}

/// Which limit a value tripped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Breach {
    WarnAbove(f64),
    WarnBelow(f64),
    DownAbove(f64),
    DownBelow(f64),
}

impl Breach {
    pub fn is_down(self) -> bool {
        matches!(self, Self::DownAbove(_) | Self::DownBelow(_))
    }
}

impl Thresholds {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Down limits must not be milder than warning limits, otherwise a warning could never show.
    pub fn validate(&self) -> Result<(), Msg> {
        let all = [
            self.warn_above,
            self.warn_below,
            self.down_above,
            self.down_below,
        ];
        if all.iter().flatten().any(|v| !v.is_finite()) {
            return Err(Msg::new("thresholds-not-finite"));
        }
        if let (Some(w), Some(d)) = (self.warn_above, self.down_above)
            && d < w
        {
            return Err(Msg::new("thresholds-down-above-below-warn"));
        }
        if let (Some(w), Some(d)) = (self.warn_below, self.down_below)
            && d > w
        {
            return Err(Msg::new("thresholds-down-below-above-warn"));
        }
        let upper = self.warn_above.or(self.down_above);
        let lower = self.warn_below.or(self.down_below);
        if let (Some(u), Some(l)) = (upper, lower)
            && u < l
        {
            return Err(Msg::new("thresholds-upper-below-lower"));
        }
        Ok(())
    }

    /// The most severe limit that `value` trips, if any.
    pub fn breach(&self, value: f64) -> Option<Breach> {
        if let Some(l) = self.down_above.filter(|l| value > *l) {
            return Some(Breach::DownAbove(l));
        }
        if let Some(l) = self.down_below.filter(|l| value < *l) {
            return Some(Breach::DownBelow(l));
        }
        if let Some(l) = self.warn_above.filter(|l| value > *l) {
            return Some(Breach::WarnAbove(l));
        }
        self.warn_below
            .filter(|l| value < *l)
            .map(Breach::WarnBelow)
    }
}

/// What one poll says about the sensor.
#[derive(Debug, Clone, PartialEq)]
pub enum Assessment {
    /// The measurement worked and the value is fine.
    Healthy(Msg),
    /// The measurement worked but a warning limit is exceeded.
    Degraded(Msg),
    /// The measurement worked but the value says the monitored thing is down (interface down,
    /// down limit exceeded).
    Unhealthy(Msg),
    /// The measurement itself failed (timeout, unreachable, unusable value).
    Failed(Msg),
}

/// A status message for the UI or a mail, cut to a readable length.
pub fn shown(t: Lang, message: &Msg) -> String {
    truncate(t.msg(message))
}

fn truncate(mut s: String) -> String {
    if s.chars().count() > MAX_MESSAGE_CHARS {
        s = s.chars().take(MAX_MESSAGE_CHARS - 1).collect::<String>() + "…";
    }
    s
}

impl SensorError {
    /// Explanation for the UI; never contains secrets. Texts from the device are cut short.
    pub fn user_message(&self) -> Msg {
        match self {
            Self::Snmp(e) => e.user_message(),
            Self::BadValue(m) => Msg::new("sensor-bad-value").with("detail", truncate(m.clone())),
            Self::NoSession => Msg::new("sensor-no-session"),
            Self::Unreachable(m) => {
                Msg::new("sensor-unreachable").with("detail", truncate(m.clone()))
            }
            Self::Check(m) => m.clone(),
        }
    }
}

fn oper_status_name(code: i64) -> &'static str {
    match code {
        1 => "up",
        2 => "down",
        3 => "testing",
        4 => "unknown",
        5 => "dormant",
        6 => "notPresent",
        7 => "lowerLayerDown",
        _ => "?",
    }
}

/// How one value of an enumeration is rated, e.g. PoE status `overload(3)` → [`Status::Down`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueState {
    pub value: i64,
    /// Name of the value from the MIB (kept for messages, the poller has no MIB index).
    pub label: String,
    /// `Up`, `Warning` or `Down`.
    pub status: Status,
}

/// What decides the state of one channel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChannelRules {
    pub thresholds: Thresholds,
    /// Ratings of status values; empty = status values are not rated.
    pub states: Vec<ValueState>,
}

/// Words in the name of an enumeration value that mean a failure or a warning, checked as parts
/// of the lowercased name without `-`/`_` (`hardware-fault` → `hardwarefault`).
const DOWN_WORDS: &[&str] = &[
    "fail",
    "fault",
    "critical",
    "error",
    "nonoperational",
    "alarm",
    "broken",
    "overload",
    "short",
    "overtemp",
    "overheat",
    "notpresent",
    "lowerlayerdown",
    "dead",
    "lost",
];
const WARNING_WORDS: &[&str] = &[
    "warn",
    "degrad",
    "minor",
    "unavailable",
    "testing",
    "nonstandard",
    "voltagehigh",
    "voltagelow",
    "highvoltage",
    "lowvoltage",
];

/// A first guess for the rating of an enumeration value from its name: clear failure words are
/// `Down`, clear warning words `Warning`, everything else (`ok`, `on`, `off`, `enabled`, …) `Up`.
/// Negations like `noError` stay `Up`.
pub fn suggest_status(label: &str) -> Status {
    let name: String = label
        .chars()
        .filter(|c| !matches!(c, '-' | '_' | ' '))
        .flat_map(char::to_lowercase)
        .collect();
    let negated = |word: &str| name.starts_with("no") && name[2..].starts_with(word);
    let hit = |words: &[&str]| words.iter().any(|w| name.contains(w) && !negated(w));
    if name == "down" || name == "bad" || name == "major" || hit(DOWN_WORDS) {
        Status::Down
    } else if name == "unknown" || hit(WARNING_WORDS) {
        Status::Warning
    } else {
        Status::Up
    }
}

/// Judges a poll result. `limits` gives the thresholds of a channel by key.
pub fn assess(
    spec: &SensorSpec,
    result: &Result<Measurement, SensorError>,
    limits: &dyn Fn(&str) -> Thresholds,
) -> Assessment {
    assess_with(spec, result, &|key| ChannelRules {
        thresholds: limits(key),
        states: Vec::new(),
    })
}

/// Judges a poll result with thresholds and ratings of status values per channel. The worst
/// finding wins; without a rating, an interface status other than up is a failure.
pub fn assess_with(
    spec: &SensorSpec,
    result: &Result<Measurement, SensorError>,
    rules: &dyn Fn(&str) -> ChannelRules,
) -> Assessment {
    let measurement = match result {
        Ok(m) => m,
        Err(e) => return Assessment::Failed(e.user_message()),
    };
    let interface_status = matches!(spec, SensorSpec::InterfaceStatus { .. });
    if interface_status && measurement.channels.iter().all(|c| c.value.is_none()) {
        return Assessment::Failed(Msg::new("assess-no-status-value"));
    }
    // (severity: 1 = warning, 2 = down, message); the first finding of the worst severity wins.
    let mut worst: Option<(u8, Msg)> = None;
    let mut consider = |level: u8, message: Msg| {
        if worst.as_ref().is_none_or(|(l, _)| level > *l) {
            worst = Some((level, message));
        }
    };
    let mut fine: Vec<Msg> = Vec::new();
    for reading in &measurement.channels {
        let Some(value) = reading.value else { continue };
        let rules = rules(&reading.key);
        if !rules.states.is_empty() {
            match rules.states.iter().find(|s| s.value as f64 == value) {
                Some(s) => {
                    let shown = format!("{} ({})", s.label, s.value);
                    match s.status {
                        Status::Down => {
                            consider(2, Msg::new("assess-state-down").with("state", shown))
                        }
                        Status::Warning => {
                            consider(1, Msg::new("assess-state-warning").with("state", shown))
                        }
                        _ => fine.push(Msg::Text(shown)),
                    }
                }
                None => consider(
                    1,
                    Msg::new("assess-state-unknown").with_value("value", reading.unit, value),
                ),
            }
        } else if interface_status {
            if value == 1.0 {
                fine.push(Msg::new("assess-interface-up"));
            } else {
                consider(
                    2,
                    Msg::new("assess-interface-not-up")
                        .with("status", oper_status_name(value as i64)),
                );
            }
        }
        if let Some(breach) = rules.thresholds.breach(value) {
            let (id, limit) = match breach {
                Breach::WarnAbove(l) => ("assess-above-warn", l),
                Breach::WarnBelow(l) => ("assess-below-warn", l),
                Breach::DownAbove(l) => ("assess-above-down", l),
                Breach::DownBelow(l) => ("assess-below-down", l),
            };
            consider(
                if breach.is_down() { 2 } else { 1 },
                Msg::new(id)
                    .with("channel", channel_label(&reading.key))
                    .with_value("value", reading.unit, value)
                    .with_value("limit", reading.unit, limit),
            );
        }
    }
    match worst {
        Some((2, message)) => Assessment::Unhealthy(message),
        Some((_, message)) => Assessment::Degraded(message),
        None if fine.is_empty() => Assessment::Healthy(
            measurement
                .note
                .clone()
                .unwrap_or_else(|| Msg::new("assess-ok")),
        ),
        None => Assessment::Healthy(Msg::List(fine)),
    }
}

/// The new state after a poll at `now_ms`, given the previous state (if any).
pub fn advance(prev: Option<&SensorState>, assessment: &Assessment, now_ms: i64) -> SensorState {
    let (status, failures, message) = match assessment {
        Assessment::Healthy(m) => (Status::Up, 0, m),
        Assessment::Degraded(m) => (Status::Warning, 0, m),
        Assessment::Unhealthy(m) => (Status::Down, 0, m),
        Assessment::Failed(m) => {
            let failures = prev.map_or(0, |p| p.consecutive_failures).saturating_add(1);
            let status = if failures >= DOWN_AFTER_FAILURES {
                Status::Down
            } else {
                Status::Warning
            };
            (status, failures, m)
        }
    };
    let since_ms = match prev {
        Some(p) if p.status == status => p.since_ms,
        _ => now_ms,
    };
    SensorState {
        status,
        since_ms,
        last_poll_ms: now_ms,
        message: message.clone(),
        consecutive_failures: failures,
    }
}

/// The status to show for a sensor: `Paused` while it is paused (by itself, its device or a group);
/// after a pause its stored "paused" means `Unknown` until the next poll.
pub fn shown_status(paused: bool, stored: Option<Status>) -> Status {
    match stored {
        _ if paused => Status::Paused,
        None | Some(Status::Paused) => Status::Unknown,
        Some(status) => status,
    }
}

/// Device status from its sensors, given as `(active, stored state)` (active = not paused). Paused
/// sensors are ignored; a device with only paused sensors is `Paused`, one without sensors or
/// without any poll yet is `Unknown`. The worst status wins.
pub fn device_status(sensors: impl IntoIterator<Item = (bool, Option<Status>)>) -> Status {
    let mut any = false;
    let mut worst: Option<Status> = None;
    for (active, state) in sensors {
        any = true;
        if !active {
            continue;
        }
        let state = shown_status(false, state);
        if worst.is_none_or(|w| state.severity() > w.severity()) {
            worst = Some(state);
        }
    }
    match (any, worst) {
        (_, Some(status)) => status,
        (true, None) => Status::Paused,
        (false, None) => Status::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn de(m: &Msg) -> String {
        Lang::De.msg(m)
    }

    /// The assessment with its message as German text, for comparisons.
    fn rendered(a: Assessment) -> Assessment {
        match a {
            Assessment::Healthy(m) => Assessment::Healthy(de(&m).into()),
            Assessment::Degraded(m) => Assessment::Degraded(de(&m).into()),
            Assessment::Unhealthy(m) => Assessment::Unhealthy(de(&m).into()),
            Assessment::Failed(m) => Assessment::Failed(de(&m).into()),
        }
    }
    use crate::sensors::{ChannelReading, Unit};
    use crate::snmp::SnmpError;

    fn none(_: &str) -> Thresholds {
        Thresholds::default()
    }

    fn failed() -> Assessment {
        Assessment::Failed("x".into())
    }

    fn healthy() -> Assessment {
        Assessment::Healthy("OK".into())
    }

    #[test]
    fn first_success_is_up_since_now() {
        let s = advance(None, &healthy(), 5_000);
        assert_eq!(
            (s.status, s.since_ms, s.last_poll_ms, s.consecutive_failures),
            (Status::Up, 5_000, 5_000, 0)
        );
    }

    #[test]
    fn failures_warn_first_then_go_down() {
        let mut s = advance(None, &healthy(), 0);
        s = advance(Some(&s), &failed(), 10);
        assert_eq!(
            (s.status, s.consecutive_failures, s.since_ms),
            (Status::Warning, 1, 10)
        );
        s = advance(Some(&s), &failed(), 20);
        assert_eq!(
            (s.status, s.consecutive_failures, s.since_ms),
            (Status::Down, 2, 20)
        );
        // Staying down keeps the original "since".
        s = advance(Some(&s), &failed(), 30);
        assert_eq!(
            (s.status, s.consecutive_failures, s.since_ms),
            (Status::Down, 3, 20)
        );
    }

    #[test]
    fn one_success_recovers_immediately() {
        let mut s = advance(None, &failed(), 0);
        s = advance(Some(&s), &failed(), 10);
        s = advance(Some(&s), &healthy(), 20);
        assert_eq!(
            (s.status, s.consecutive_failures, s.since_ms),
            (Status::Up, 0, 20)
        );
    }

    #[test]
    fn a_single_failure_between_successes_never_goes_down() {
        let mut s = advance(None, &healthy(), 0);
        for t in 1..10 {
            s = advance(Some(&s), &failed(), t * 10);
            assert_eq!(s.status, Status::Warning);
            s = advance(Some(&s), &healthy(), t * 10 + 5);
            assert_eq!(s.status, Status::Up);
        }
    }

    #[test]
    fn unhealthy_reading_is_down_without_debounce() {
        let s = advance(None, &Assessment::Unhealthy("down".into()), 1);
        assert_eq!((s.status, s.consecutive_failures), (Status::Down, 0));
    }

    fn status_reading(value: Option<f64>) -> Result<Measurement, SensorError> {
        Ok(Measurement {
            note: None,
            channels: vec![ChannelReading {
                key: "oper_status".to_owned(),
                unit: Unit::State,
                value,
            }],
        })
    }

    #[test]
    fn interface_status_is_judged_by_oper_status() {
        let spec = SensorSpec::InterfaceStatus { if_index: 1 };
        assert!(matches!(
            assess(&spec, &status_reading(Some(1.0)), &none),
            Assessment::Healthy(_)
        ));
        let down = assess(&spec, &status_reading(Some(2.0)), &none);
        assert!(
            matches!(&down, Assessment::Unhealthy(m) if de(m).contains("down")),
            "{down:?}"
        );
        let lower = assess(&spec, &status_reading(Some(7.0)), &none);
        assert!(matches!(&lower, Assessment::Unhealthy(m) if de(m).contains("lowerLayerDown")));
        assert!(matches!(
            assess(&spec, &status_reading(None), &none),
            Assessment::Failed(_)
        ));
    }

    #[test]
    fn other_sensors_are_healthy_when_they_deliver() {
        let ok = Ok(Measurement::default());
        assert!(matches!(
            assess(&SensorSpec::Uptime, &ok, &none),
            Assessment::Healthy(_)
        ));
        assert!(matches!(
            assess(&SensorSpec::ping(), &ok, &none),
            Assessment::Healthy(_)
        ));
    }

    #[test]
    fn errors_fail_with_a_german_message() {
        let err = Err(SensorError::Snmp(SnmpError::Timeout));
        let a = assess(&SensorSpec::Uptime, &err, &none);
        assert!(
            matches!(&a, Assessment::Failed(m) if de(m).contains("Keine Antwort")),
            "{a:?}"
        );
        let err = Err(SensorError::Unreachable("x".to_owned()));
        assert!(
            matches!(assess(&SensorSpec::ping(), &err, &none), Assessment::Failed(m) if de(&m).starts_with("Nicht erreichbar"))
        );
    }

    #[test]
    fn long_messages_are_truncated() {
        let err = Err(SensorError::BadValue("ä".repeat(1_000)));
        let Assessment::Failed(m) = assess(&SensorSpec::Uptime, &err, &none) else {
            panic!("expected failure");
        };
        let text = shown(Lang::De, &m);
        assert_eq!(text.chars().count(), MAX_MESSAGE_CHARS);
        assert!(text.ends_with('…'));
        assert!(m.encode().chars().count() < 2 * MAX_MESSAGE_CHARS);
    }

    #[test]
    fn device_status_takes_the_worst_enabled_sensor() {
        use Status::*;
        assert_eq!(device_status([(true, Some(Up)), (true, Some(Up))]), Up);
        assert_eq!(
            device_status([(true, Some(Up)), (true, Some(Warning))]),
            Warning
        );
        assert_eq!(
            device_status([(true, Some(Warning)), (true, Some(Down)), (true, Some(Up))]),
            Down
        );
        assert_eq!(device_status([(true, Some(Up)), (true, None)]), Unknown);
        assert_eq!(device_status([(true, Some(Up)), (false, Some(Down))]), Up);
        assert_eq!(device_status([(false, Some(Down))]), Paused);
        assert_eq!(device_status([]), Unknown);
    }

    fn rtt(value: f64) -> Result<Measurement, SensorError> {
        Ok(Measurement {
            note: None,
            channels: vec![ChannelReading {
                key: "rtt".to_owned(),
                unit: Unit::Milliseconds,
                value: Some(value),
            }],
        })
    }

    fn rtt_limits(_: &str) -> Thresholds {
        Thresholds {
            warn_above: Some(50.0),
            down_above: Some(200.0),
            ..Thresholds::default()
        }
    }

    #[test]
    fn thresholds_turn_values_into_warning_and_down() {
        let spec = SensorSpec::ping();
        assert!(matches!(
            assess(&spec, &rtt(10.0), &rtt_limits),
            Assessment::Healthy(_)
        ));
        assert!(
            matches!(
                assess(&spec, &rtt(50.0), &rtt_limits),
                Assessment::Healthy(_)
            ),
            "limits are exclusive"
        );
        let warn = assess(&spec, &rtt(80.0), &rtt_limits);
        assert!(
            matches!(&warn, Assessment::Degraded(m) if de(m) == "Antwortzeit: 80 ms liegt über der Warngrenze 50 ms"),
            "{warn:?}"
        );
        let down = assess(&spec, &rtt(250.0), &rtt_limits);
        assert!(
            matches!(&down, Assessment::Unhealthy(m) if de(m).contains("Ausfallgrenze 200 ms")),
            "{down:?}"
        );
    }

    #[test]
    fn a_down_breach_beats_a_warning_on_another_channel() {
        let m = Ok(Measurement {
            note: None,
            channels: vec![
                ChannelReading {
                    key: "in".to_owned(),
                    unit: Unit::BitsPerSecond,
                    value: Some(2_000_000.0),
                },
                ChannelReading {
                    key: "out".to_owned(),
                    unit: Unit::BitsPerSecond,
                    value: Some(9_000_000.0),
                },
                ChannelReading {
                    key: "gone".to_owned(),
                    unit: Unit::BitsPerSecond,
                    value: None,
                },
            ],
        });
        let limits = |key: &str| match key {
            "in" => Thresholds {
                warn_above: Some(1_000_000.0),
                ..Thresholds::default()
            },
            "out" => Thresholds {
                down_above: Some(8_000_000.0),
                ..Thresholds::default()
            },
            _ => Thresholds {
                down_above: Some(0.0),
                ..Thresholds::default()
            },
        };
        let a = assess(&SensorSpec::InterfaceTraffic { if_index: 1 }, &m, &limits);
        assert!(
            matches!(&a, Assessment::Unhealthy(m) if de(m).starts_with("Ausgehend: 9 Mbit/s")),
            "{a:?}"
        );
    }

    #[test]
    fn degraded_polls_warn_without_debounce_and_recover() {
        let s = advance(None, &Assessment::Degraded("hoch".into()), 5);
        assert_eq!((s.status, s.consecutive_failures), (Status::Warning, 0));
        let s = advance(Some(&s), &healthy(), 6);
        assert_eq!(s.status, Status::Up);
    }

    #[test]
    fn threshold_validation_and_breach_logic() {
        let t = |wa, wb, da, db| Thresholds {
            warn_above: wa,
            warn_below: wb,
            down_above: da,
            down_below: db,
        };
        assert!(Thresholds::default().validate().is_ok() && Thresholds::default().is_empty());
        assert!(t(Some(1.0), None, Some(2.0), None).validate().is_ok());
        assert!(t(Some(2.0), None, Some(1.0), None).validate().is_err());
        assert!(t(None, Some(1.0), None, Some(2.0)).validate().is_err());
        assert!(
            t(Some(1.0), Some(5.0), None, None).validate().is_err(),
            "upper below lower"
        );
        assert!(t(Some(f64::NAN), None, None, None).validate().is_err());
        assert!(
            t(Some(10.0), Some(1.0), Some(20.0), Some(0.0))
                .validate()
                .is_ok()
        );

        let both = t(Some(10.0), Some(1.0), Some(20.0), Some(0.0));
        assert_eq!(both.breach(5.0), None);
        assert_eq!(both.breach(11.0), Some(Breach::WarnAbove(10.0)));
        assert_eq!(both.breach(0.5), Some(Breach::WarnBelow(1.0)));
        assert_eq!(both.breach(21.0), Some(Breach::DownAbove(20.0)));
        assert_eq!(both.breach(-1.0), Some(Breach::DownBelow(0.0)));
    }

    #[test]
    fn status_names_round_trip() {
        for s in [
            Status::Up,
            Status::Warning,
            Status::Down,
            Status::Unknown,
            Status::Paused,
        ] {
            assert_eq!(Status::parse(s.as_str()), Some(s));
        }
        assert_eq!(Status::parse("broken"), None);
    }

    #[test]
    fn names_of_status_values_suggest_a_rating() {
        let poe = [
            ("off", Status::Up),
            ("turning-on", Status::Up),
            ("on", Status::Up),
            ("overload", Status::Down),
            ("short", Status::Down),
            ("nonstandard-pd", Status::Warning),
            ("voltage-high", Status::Warning),
            ("voltage-low", Status::Warning),
            ("hardware-fault", Status::Down),
            ("overtemperature", Status::Down),
        ];
        let others = [
            ("up", Status::Up),
            ("down", Status::Down),
            ("testing", Status::Warning),
            ("unknown", Status::Warning),
            ("dormant", Status::Up),
            ("notPresent", Status::Down),
            ("lowerLayerDown", Status::Down),
            ("ok", Status::Up),
            ("unavailable", Status::Warning),
            ("nonoperational", Status::Down),
            ("noError", Status::Up),
            ("no-fault", Status::Up),
            ("enable", Status::Up),
            ("disable", Status::Up),
            ("criticalAlarm", Status::Down),
            ("Lüfter-Fehler", Status::Up),
        ];
        for (label, expected) in poe.into_iter().chain(others) {
            assert_eq!(suggest_status(label), expected, "{label}");
        }
    }

    fn gauge(value: f64) -> Result<Measurement, SensorError> {
        Ok(Measurement {
            note: None,
            channels: vec![ChannelReading {
                key: "value".to_owned(),
                unit: Unit::Count,
                value: Some(value),
            }],
        })
    }

    fn poe_rules(_: &str) -> ChannelRules {
        let state = |value, label: &str, status| ValueState {
            value,
            label: label.to_owned(),
            status,
        };
        ChannelRules {
            thresholds: Thresholds::default(),
            states: vec![
                state(0, "off", Status::Up),
                state(2, "on", Status::Up),
                state(3, "overload", Status::Down),
                state(7, "voltage-low", Status::Warning),
            ],
        }
    }

    #[test]
    fn rated_status_values_decide_the_state() {
        let spec = SensorSpec::Oid {
            oid: vec![1, 3, 6, 1, 4, 1, 9, 0],
            mode: crate::sensors::snmp::OidMode::Gauge,
            scale: 1.0,
            unit: None,
        };
        assert_eq!(
            rendered(assess_with(&spec, &gauge(2.0), &poe_rules)),
            Assessment::Healthy("on (2)".into())
        );
        assert_eq!(
            rendered(assess_with(&spec, &gauge(3.0), &poe_rules)),
            Assessment::Unhealthy("overload (3) gilt als Ausfall".into())
        );
        assert_eq!(
            rendered(assess_with(&spec, &gauge(7.0), &poe_rules)),
            Assessment::Degraded("voltage-low (7) gilt als Warnung".into())
        );
        assert!(matches!(
            assess_with(&spec, &gauge(42.0), &poe_rules),
            Assessment::Degraded(m) if de(&m).starts_with("Unbekannter Statuswert 42")
        ));
        // Without ratings a status value is just a number.
        assert_eq!(
            rendered(assess(&spec, &gauge(3.0), &none)),
            Assessment::Healthy("OK".into())
        );
    }

    #[test]
    fn rated_interface_status_replaces_the_built_in_rule() {
        let spec = SensorSpec::InterfaceStatus { if_index: 1 };
        let mild = |_: &str| ChannelRules {
            thresholds: Thresholds::default(),
            states: vec![
                ValueState {
                    value: 1,
                    label: "up".to_owned(),
                    status: Status::Up,
                },
                ValueState {
                    value: 2,
                    label: "down".to_owned(),
                    status: Status::Warning,
                },
            ],
        };
        assert_eq!(
            rendered(assess_with(&spec, &status_reading(Some(2.0)), &mild)),
            Assessment::Degraded("down (2) gilt als Warnung".into())
        );
        assert!(matches!(
            assess_with(&spec, &status_reading(Some(2.0)), &|_| {
                ChannelRules::default()
            }),
            Assessment::Unhealthy(_)
        ));
        assert!(matches!(
            assess_with(&spec, &status_reading(None), &mild),
            Assessment::Failed(_)
        ));
    }
}
