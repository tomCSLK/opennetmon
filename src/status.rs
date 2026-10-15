//! Status logic: turns poll results into sensor states and derives device states.
//!
//! Pure functions only; persistence is in `storage::status`. Threshold evaluation per channel
//! (warning/down limits) plugs into [`assess`] later.

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
            Self::Paused => "Pausiert",
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

/// What one poll says about the sensor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assessment {
    /// The measurement worked and the value is fine.
    Healthy(String),
    /// The measurement worked but the value says the monitored thing is down (interface down).
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

pub fn assess(spec: &SensorSpec, result: &Result<Measurement, SensorError>) -> Assessment {
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
    Assessment::Healthy("OK".to_owned())
}

/// The new state after a poll at `now_ms`, given the previous state (if any).
pub fn advance(prev: Option<&SensorState>, assessment: &Assessment, now_ms: i64) -> SensorState {
    let (status, failures, message) = match assessment {
        Assessment::Healthy(m) => (Status::Up, 0, m),
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
            assess(&spec, &status_reading(Some(1.0))),
            Assessment::Healthy(_)
        ));
        let down = assess(&spec, &status_reading(Some(2.0)));
        assert!(
            matches!(&down, Assessment::Unhealthy(m) if m.contains("down")),
            "{down:?}"
        );
        let lower = assess(&spec, &status_reading(Some(7.0)));
        assert!(matches!(&lower, Assessment::Unhealthy(m) if m.contains("lowerLayerDown")));
        assert!(matches!(
            assess(&spec, &status_reading(None)),
            Assessment::Failed(_)
        ));
    }

    #[test]
    fn other_sensors_are_healthy_when_they_deliver() {
        let ok = Ok(Measurement::default());
        assert!(matches!(
            assess(&SensorSpec::Uptime, &ok),
            Assessment::Healthy(_)
        ));
        assert!(matches!(
            assess(&SensorSpec::Ping, &ok),
            Assessment::Healthy(_)
        ));
    }

    #[test]
    fn errors_fail_with_a_german_message() {
        let err = Err(SensorError::Snmp(SnmpError::Timeout));
        let a = assess(&SensorSpec::Uptime, &err);
        assert!(
            matches!(&a, Assessment::Failed(m) if m.contains("Keine Antwort")),
            "{a:?}"
        );
        let err = Err(SensorError::Unreachable("x".to_owned()));
        assert!(
            matches!(assess(&SensorSpec::Ping, &err), Assessment::Failed(m) if m.starts_with("Nicht erreichbar"))
        );
    }

    #[test]
    fn long_messages_are_truncated() {
        let err = Err(SensorError::BadValue("ä".repeat(1_000)));
        let Assessment::Failed(m) = assess(&SensorSpec::Uptime, &err) else {
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
