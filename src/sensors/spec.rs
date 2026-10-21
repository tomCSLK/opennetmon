//! Sensor configuration: what the user chose, validation, channel layout and construction.
//! Messages are user-facing (German).

use super::local::{LocalMetric, LocalSensor, Paths};
use super::ping::PingSensor;
use super::snmp::{
    InterfaceStatusSensor, InterfaceTrafficSensor, OidMode, OidSensor, UptimeSensor,
};
use super::{Sensor, Unit};
use crate::i18n::{Lang, Msg};

pub const MAX_OID_ARCS: usize = 128;
/// Largest accepted `ifIndex` (the value is a 31-bit integer in the IF-MIB).
pub const MAX_IF_INDEX: u32 = i32::MAX as u32;

#[derive(Debug, Clone, PartialEq)]
pub enum SensorSpec {
    Ping,
    Uptime,
    InterfaceTraffic {
        if_index: u32,
    },
    InterfaceStatus {
        if_index: u32,
    },
    Oid {
        oid: Vec<u64>,
        mode: OidMode,
        scale: f64,
        /// Unit shown with the value, e.g. from the MIB's UNITS clause (`°C`, `KBytes`).
        unit: Option<String>,
    },
    /// A value of the machine opennetmon runs on ("Dieser Server").
    Local(LocalMetric),
}

/// Longest accepted unit label of a custom OID sensor.
pub const MAX_UNIT_CHARS: usize = 24;

/// One channel a sensor produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelDef {
    pub key: &'static str,
    pub unit: Unit,
}

impl Unit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BitsPerSecond => "bps",
            Self::Milliseconds => "ms",
            Self::Seconds => "s",
            Self::Percent => "percent",
            Self::Count => "count",
            Self::PerSecond => "per_second",
            Self::State => "state",
            Self::Bytes => "bytes",
            Self::Celsius => "celsius",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "bps" => Self::BitsPerSecond,
            "ms" => Self::Milliseconds,
            "s" => Self::Seconds,
            "percent" => Self::Percent,
            "count" => Self::Count,
            "per_second" => Self::PerSecond,
            "state" => Self::State,
            "bytes" => Self::Bytes,
            "celsius" => Self::Celsius,
            _ => return None,
        })
    }
}

impl SensorSpec {
    /// Stable identifier stored in the database.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Ping => "ping",
            Self::Uptime => "uptime",
            Self::InterfaceTraffic { .. } => "if_traffic",
            Self::InterfaceStatus { .. } => "if_status",
            Self::Oid { .. } => "oid",
            Self::Local(_) => "local",
        }
    }

    /// Short description for lists, e.g. `Interface-Traffic (Index 49153)`.
    pub fn describe(&self, t: Lang) -> String {
        match self {
            Self::Ping => "Ping".to_owned(),
            Self::Uptime => "Uptime".to_owned(),
            Self::InterfaceTraffic { if_index } => {
                t.get1("sensor-describe-traffic", "index", *if_index)
            }
            Self::InterfaceStatus { if_index } => {
                t.get1("sensor-describe-status", "index", *if_index)
            }
            Self::Oid { oid, .. } => format!("OID {} ({})", format_oid(oid), self.oid_details(t)),
            Self::Local(metric) => match metric {
                LocalMetric::Cpu => t.get("local-describe-cpu"),
                LocalMetric::Load => t.get("local-describe-load"),
                LocalMetric::Memory => t.get("local-describe-memory"),
                LocalMetric::Uptime => t.get("local-describe-uptime"),
                LocalMetric::Disk { mount } => t.get1("local-describe-disk", "mount", mount),
                LocalMetric::Temperature { sensor } => {
                    t.get1("local-describe-temperature", "sensor", sensor)
                }
                LocalMetric::Traffic { interface } => {
                    t.get1("local-describe-traffic", "interface", interface)
                }
            },
        }
    }

    /// Mode, factor and unit of a custom OID sensor, e.g. `Wert, Faktor 0.1, Einheit °C`.
    pub fn oid_details(&self, t: Lang) -> String {
        let Self::Oid {
            mode, scale, unit, ..
        } = self
        else {
            return String::new();
        };
        let mut text = t.get(match mode {
            OidMode::Gauge => "oid-mode-gauge",
            OidMode::Counter => "oid-mode-counter",
        });
        if *scale != 1.0 {
            text.push_str(&t.get1("oid-detail-factor", "factor", scale.to_string()));
        }
        if let Some(u) = unit {
            text.push_str(&t.get1("oid-detail-unit", "unit", u));
        }
        text
    }

    pub fn channels(&self) -> Vec<ChannelDef> {
        let one = |key, unit| vec![ChannelDef { key, unit }];
        match self {
            Self::Ping => one("rtt", Unit::Milliseconds),
            Self::Uptime => one("uptime", Unit::Seconds),
            Self::InterfaceTraffic { .. } => vec![
                ChannelDef {
                    key: "in",
                    unit: Unit::BitsPerSecond,
                },
                ChannelDef {
                    key: "out",
                    unit: Unit::BitsPerSecond,
                },
            ],
            Self::InterfaceStatus { .. } => one("oper_status", Unit::State),
            Self::Oid {
                mode: OidMode::Gauge,
                ..
            } => one("value", Unit::Count),
            Self::Oid {
                mode: OidMode::Counter,
                ..
            } => one("value", Unit::PerSecond),
            Self::Local(metric) => metric.channels(),
        }
    }

    /// Whether the sensor needs an SNMP session (ping and local sensors do not).
    pub fn uses_snmp(&self) -> bool {
        !matches!(self, Self::Ping | Self::Local(_))
    }

    pub fn validate(&self) -> Result<(), Msg> {
        match self {
            Self::InterfaceTraffic { if_index } | Self::InterfaceStatus { if_index }
                if *if_index == 0 || *if_index > MAX_IF_INDEX =>
            {
                Err(Msg::new("sensor-if-index-invalid"))
            }
            Self::Oid {
                oid, scale, unit, ..
            } => {
                validate_oid(oid)?;
                if !scale.is_finite() || *scale == 0.0 {
                    return Err(Msg::new("sensor-factor-invalid"));
                }
                if let Some(u) = unit
                    && (u.trim().is_empty()
                        || u.chars().count() > MAX_UNIT_CHARS
                        || u.chars().any(char::is_control))
                {
                    return Err(Msg::new("sensor-unit-invalid"));
                }
                Ok(())
            }
            Self::Local(metric) => metric.validate(),
            _ => Ok(()),
        }
    }

    /// A fresh sensor instance (counter state starts empty).
    pub fn build(&self) -> Box<dyn Sensor> {
        match self {
            Self::Ping => Box::new(PingSensor::new()),
            Self::Uptime => Box::new(UptimeSensor),
            Self::InterfaceTraffic { if_index } => Box::new(InterfaceTrafficSensor::new(*if_index)),
            Self::InterfaceStatus { if_index } => Box::new(InterfaceStatusSensor::new(*if_index)),
            Self::Oid {
                oid, mode, scale, ..
            } => Box::new(OidSensor::new(oid.clone(), *mode, *scale)),
            Self::Local(metric) => Box::new(LocalSensor::new(metric.clone(), Paths::default())),
        }
    }
}

fn validate_oid(oid: &[u64]) -> Result<(), Msg> {
    match oid {
        [first, second, ..]
            if oid.len() <= MAX_OID_ARCS
                && *first <= 2
                && (*first == 2 || *second < 40)
                && oid.iter().all(|a| *a <= u64::from(u32::MAX)) =>
        {
            Ok(())
        }
        _ => Err(Msg::new("oid-invalid")),
    }
}

/// Parses a numeric dotted OID such as `1.3.6.1.2.1.1.3.0` (a leading dot is tolerated).
pub fn parse_oid(text: &str) -> Result<Vec<u64>, Msg> {
    let invalid = || Msg::new("oid-not-numeric");
    let oid: Vec<u64> = text
        .trim()
        .trim_start_matches('.')
        .split('.')
        .map(|part| {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid());
            }
            part.parse::<u64>().map_err(|_| invalid())
        })
        .collect::<Result<_, _>>()?;
    validate_oid(&oid)?;
    Ok(oid)
}

pub fn format_oid(oid: &[u64]) -> String {
    oid.iter().map(u64::to_string).collect::<Vec<_>>().join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numeric_oids() {
        assert_eq!(
            parse_oid("1.3.6.1.2.1.1.3.0").unwrap(),
            [1, 3, 6, 1, 2, 1, 1, 3, 0]
        );
        assert_eq!(parse_oid(" .1.3.6.1 ").unwrap(), [1, 3, 6, 1]);
        assert_eq!(format_oid(&[1, 3, 6]), "1.3.6");
    }

    #[test]
    fn rejects_bad_oids() {
        for bad in [
            "",
            "1",
            "1.",
            "1..3",
            "a.b",
            "1.3.x",
            "-1.3",
            "3.1.2",
            "1.40.1",
            "1.3.99999999999",
            "sysUpTime.0",
            "1.3.6 .1",
        ] {
            assert!(parse_oid(bad).is_err(), "{bad:?} should be rejected");
        }
        let long = vec!["1"; MAX_OID_ARCS + 1].join(".");
        assert!(parse_oid(&long).is_err());
    }

    #[test]
    fn validates_interface_index_and_scale() {
        assert!(
            SensorSpec::InterfaceTraffic { if_index: 49153 }
                .validate()
                .is_ok()
        );
        assert!(
            SensorSpec::InterfaceStatus { if_index: 0 }
                .validate()
                .is_err()
        );
        assert!(
            SensorSpec::InterfaceStatus {
                if_index: MAX_IF_INDEX + 1
            }
            .validate()
            .is_err()
        );
        let oid = |scale| SensorSpec::Oid {
            oid: vec![1, 3, 6, 1],
            mode: OidMode::Gauge,
            scale,
            unit: None,
        };
        let with_unit = |unit: &str| SensorSpec::Oid {
            oid: vec![1, 3, 6, 1],
            mode: OidMode::Gauge,
            scale: 1.0,
            unit: Some(unit.to_owned()),
        };
        assert!(with_unit("°C").validate().is_ok());
        assert!(with_unit("").validate().is_err());
        assert!(with_unit(&"x".repeat(25)).validate().is_err());
        assert!(with_unit("a\nb").validate().is_err());
        assert!(oid(0.1).validate().is_ok());
        assert!(oid(0.0).validate().is_err());
        assert!(oid(f64::NAN).validate().is_err());
        assert!(oid(f64::INFINITY).validate().is_err());
    }

    #[test]
    fn channel_layout_matches_kind() {
        let keys = |s: SensorSpec| s.channels().iter().map(|c| c.key).collect::<Vec<_>>();
        assert_eq!(keys(SensorSpec::Ping), ["rtt"]);
        assert_eq!(
            keys(SensorSpec::InterfaceTraffic { if_index: 1 }),
            ["in", "out"]
        );
        assert_eq!(SensorSpec::Uptime.kind(), "uptime");
        assert!(!SensorSpec::Ping.uses_snmp());
        assert!(SensorSpec::Uptime.uses_snmp());
        let disk = SensorSpec::Local(LocalMetric::Disk { mount: "/".into() });
        assert_eq!((disk.kind(), disk.uses_snmp()), ("local", false));
        assert_eq!(keys(disk), ["used", "free"]);
    }

    #[test]
    fn units_round_trip_through_their_stored_name() {
        for unit in [
            Unit::BitsPerSecond,
            Unit::Milliseconds,
            Unit::Seconds,
            Unit::Percent,
            Unit::Count,
            Unit::PerSecond,
            Unit::State,
            Unit::Bytes,
            Unit::Celsius,
        ] {
            assert_eq!(Unit::parse(unit.as_str()), Some(unit));
        }
        assert_eq!(Unit::parse("furlongs"), None);
    }
}
