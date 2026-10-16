//! Status logic: turns poll results into sensor states and derives device states.
//!
//! Pure functions only; persistence is in `storage::status`. Threshold evaluation per channel
//! (warning/down limits) plugs into [`assess`] later.

use crate::sensors::format::{channel_label, format_value};
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

    /// German label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Up => "In Ordnung",
            Self::Warning => "Warnung",
            Self::Down => "Ausfall",
            Self::Unknown => "Unbekannt",
            Self::Paused => "Inaktiv",
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SensorState {
    pub status: Status,
    /// When the sensor entered `status`.
    pub since_ms: i64,
    pub last_poll_ms: i64,
    pub message: String,
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
    pub fn validate(&self) -> Result<(), &'static str> {
        let all = [
            self.warn_above,
            self.warn_below,
            self.down_above,
            self.down_below,
        ];
        if all.iter().flatten().any(|v| !v.is_finite()) {
            return Err("Schwellwerte müssen endliche Zahlen sein.");
        }
        if let (Some(w), Some(d)) = (self.warn_above, self.down_above)
            && d < w
        {
            return Err("Die obere Down-Grenze darf nicht unter der oberen Warngrenze liegen.");
        }
        if let (Some(w), Some(d)) = (self.warn_below, self.down_below)
            && d > w
        {
            return Err("Die untere Down-Grenze darf nicht über der unteren Warngrenze liegen.");
        }
        let upper = self.warn_above.or(self.down_above);
        let lower = self.warn_below.or(self.down_below);
        if let (Some(u), Some(l)) = (upper, lower)
            && u < l
        {
            return Err("Die obere Grenze darf nicht unter der unteren Grenze liegen.");
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assessment {
    /// The measurement worked and the value is fine.
    Healthy(String),
    /// The measurement worked but a warning limit is exceeded.
    Degraded(String),
    /// The measurement worked but the value says the monitored thing is down (interface down,
    /// down limit exceeded).
    Unhealthy(String),
    /// The measurement itself failed (timeout, unreachable, unusable value).
    Failed(String),
}

fn truncate(mut s: String) -> String {
    if s.chars().count() > MAX_MESSAGE_CHARS {
        s = s.chars().take(MAX_MESSAGE_CHARS - 1).collect::<String>() + "…";
    }
    s
}

impl SensorError {
    /// Explanation for the UI (German); never contains secrets.
    pub fn user_message(&self) -> String {
        match self {
            Self::Snmp(e) => e.user_message(),
            Self::BadValue(m) => format!("Unbrauchbarer Messwert: {m}"),
            Self::NoSession => "Keine SNMP-Sitzung zum Gerät möglich.".to_owned(),
            Self::Unreachable(m) => format!("Nicht erreichbar: {m}"),
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

/// Judges a poll result. `limits` gives the thresholds of a channel by key.
pub fn assess(
    spec: &SensorSpec,
    result: &Result<Measurement, SensorError>,
    limits: &dyn Fn(&str) -> Thresholds,
) -> Assessment {
    let measurement = match result {
        Ok(m) => m,
        Err(e) => return Assessment::Failed(truncate(e.user_message())),
    };
    if let SensorSpec::InterfaceStatus { .. } = spec {
        let value = measurement.channels.iter().find_map(|c| c.value);
        return match value {
            Some(1.0) => Assessment::Healthy("Interface ist up".to_owned()),
            Some(v) => Assessment::Unhealthy(format!(
                "Interface ist nicht up (ifOperStatus = {})",
                oper_status_name(v as i64)
            )),
            None => Assessment::Failed("Kein Statuswert erhalten".to_owned()),
        };
    }
    let mut worst: Option<(Breach, String)> = None;
    for reading in &measurement.channels {
        let Some(value) = reading.value else { continue };
        let Some(breach) = limits(&reading.key).breach(value) else {
            continue;
        };
        if worst
            .as_ref()
            .is_none_or(|(w, _)| breach.is_down() && !w.is_down())
        {
            let (side, kind, limit) = match breach {
                Breach::WarnAbove(l) => ("über", "Warngrenze", l),
                Breach::WarnBelow(l) => ("unter", "Warngrenze", l),
                Breach::DownAbove(l) => ("über", "Down-Grenze", l),
                Breach::DownBelow(l) => ("unter", "Down-Grenze", l),
            };
            worst = Some((
                breach,
                format!(
                    "{}: {} liegt {side} der {kind} {}",
                    channel_label(&reading.key),
                    format_value(reading.unit, value),
                    format_value(reading.unit, limit),
                ),
            ));
        }
    }
    match worst {
        Some((breach, message)) if breach.is_down() => Assessment::Unhealthy(message),
        Some((_, message)) => Assessment::Degraded(message),
        None => Assessment::Healthy("OK".to_owned()),
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

/// Device status from its sensors, given as `(enabled, state)`. Paused (disabled) sensors are
/// ignored; a device with only paused sensors is `Paused`, one without sensors or without any
/// poll yet is `Unknown`. The worst status wins.
pub fn device_status(sensors: impl IntoIterator<Item = (bool, Option<Status>)>) -> Status {
    let mut any = false;
    let mut worst: Option<Status> = None;
    for (enabled, state) in sensors {
        any = true;
        if !enabled {
            continue;
        }
        let state = state.unwrap_or(Status::Unknown);
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
    use crate::sensors::{ChannelReading, Unit};
    use crate::snmp::SnmpError;

    fn none(_: &str) -> Thresholds {
        Thresholds::default()
    }

    fn failed() -> Assessment {
        Assessment::Failed("x".to_owned())
    }

    fn healthy() -> Assessment {
        Assessment::Healthy("OK".to_owned())
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
        let s = advance(None, &Assessment::Unhealthy("down".to_owned()), 1);
        assert_eq!((s.status, s.consecutive_failures), (Status::Down, 0));
    }

    fn status_reading(value: Option<f64>) -> Result<Measurement, SensorError> {
        Ok(Measurement {
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
            matches!(&down, Assessment::Unhealthy(m) if m.contains("down")),
            "{down:?}"
        );
        let lower = assess(&spec, &status_reading(Some(7.0)), &none);
        assert!(matches!(&lower, Assessment::Unhealthy(m) if m.contains("lowerLayerDown")));
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
            assess(&SensorSpec::Ping, &ok, &none),
            Assessment::Healthy(_)
        ));
    }

    #[test]
    fn errors_fail_with_a_german_message() {
        let err = Err(SensorError::Snmp(SnmpError::Timeout));
        let a = assess(&SensorSpec::Uptime, &err, &none);
        assert!(
            matches!(&a, Assessment::Failed(m) if m.contains("Keine Antwort")),
            "{a:?}"
        );
        let err = Err(SensorError::Unreachable("x".to_owned()));
        assert!(
            matches!(assess(&SensorSpec::Ping, &err, &none), Assessment::Failed(m) if m.starts_with("Nicht erreichbar"))
        );
    }

    #[test]
    fn long_messages_are_truncated() {
        let err = Err(SensorError::BadValue("ä".repeat(1_000)));
        let Assessment::Failed(m) = assess(&SensorSpec::Uptime, &err, &none) else {
            panic!("expected failure");
        };
        assert_eq!(m.chars().count(), MAX_MESSAGE_CHARS);
        assert!(m.ends_with('…'));
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
        let spec = SensorSpec::Ping;
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
            matches!(&warn, Assessment::Degraded(m) if m == "Antwortzeit: 80 ms liegt über der Warngrenze 50 ms"),
            "{warn:?}"
        );
        let down = assess(&spec, &rtt(250.0), &rtt_limits);
        assert!(
            matches!(&down, Assessment::Unhealthy(m) if m.contains("Down-Grenze 200 ms")),
            "{down:?}"
        );
    }

    #[test]
    fn a_down_breach_beats_a_warning_on_another_channel() {
        let m = Ok(Measurement {
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
            matches!(&a, Assessment::Unhealthy(m) if m.starts_with("Ausgehend: 9 Mbit/s")),
            "{a:?}"
        );
    }

    #[test]
    fn degraded_polls_warn_without_debounce_and_recover() {
        let s = advance(None, &Assessment::Degraded("hoch".to_owned()), 5);
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
}
