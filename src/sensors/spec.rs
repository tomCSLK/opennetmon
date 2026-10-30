//! Sensor configuration: what the user chose, validation, channel layout and construction.
//! Messages are user-facing (German).

use super::fritzbox::{FritzCheck, FritzMetric, FritzSensor};
use super::huawei::{self, HuaweiCheck, HuaweiMetric, HuaweiSensor};
use super::local::{LocalMetric, LocalSensor, Paths};
use super::ping::{PingCheck, PingSensor};
use super::service::{
    DnsCheck, DnsSensor, HttpCheck, HttpSensor, TcpCheck, TcpSensor, TlsCheck, TlsSensor,
};
use super::snmp::{
    InterfaceStatusSensor, InterfaceTrafficSensor, OidMode, OidSensor, UptimeSensor,
};
use super::{Sensor, Unit};
use crate::i18n::{Lang, Msg};
use crate::status::Thresholds;

pub const MAX_OID_ARCS: usize = 128;
/// Largest accepted `ifIndex` (the value is a 31-bit integer in the IF-MIB).
pub const MAX_IF_INDEX: u32 = i32::MAX as u32;

#[derive(Debug, Clone, PartialEq)]
pub enum SensorSpec {
    /// Echo requests to the device (several per poll).
    Ping(PingCheck),
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
    /// An HTTP(S) request to a URL.
    Http(HttpCheck),
    /// A TCP connection to a port of the device.
    Tcp(TcpCheck),
    /// A DNS question to the device as DNS server.
    Dns(DnsCheck),
    /// The TLS certificate a port of the device presents.
    Tls(TlsCheck),
    /// A value of the internet connection of a FRITZ!Box.
    Fritzbox(FritzCheck),
    /// A value of the mobile link of a Huawei router.
    Huawei(HuaweiCheck),
}

/// Packet loss above which a new ping sensor warns (percent; changeable per channel).
pub const PING_WARN_LOSS: f64 = 20.0;

/// Days before expiry at which a certificate sensor warns or fails (defaults, changeable per channel).
pub const TLS_WARN_DAYS: f64 = 14.0;
pub const TLS_DOWN_DAYS: f64 = 3.0;

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
            Self::Days => "days",
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
            "days" => Self::Days,
            _ => return None,
        })
    }
}

impl SensorSpec {
    /// A ping sensor with the default settings.
    pub fn ping() -> Self {
        Self::Ping(PingCheck::default())
    }

    /// Stable identifier stored in the database.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Ping(_) => "ping",
            Self::Uptime => "uptime",
            Self::InterfaceTraffic { .. } => "if_traffic",
            Self::InterfaceStatus { .. } => "if_status",
            Self::Oid { .. } => "oid",
            Self::Local(_) => "local",
            Self::Http(_) => "http",
            Self::Tcp(_) => "tcp",
            Self::Dns(_) => "dns",
            Self::Tls(_) => "tls",
            Self::Fritzbox(_) => "fritzbox",
            Self::Huawei(_) => "huawei",
        }
    }

    /// Settings of the ping, service and router sensors as stored in `sensors.params` (JSON);
    /// `None` for the others.
    pub fn params(&self) -> Option<String> {
        match self {
            Self::Ping(c) => serde_json::to_string(c).ok(),
            Self::Http(c) => serde_json::to_string(c).ok(),
            Self::Tcp(c) => serde_json::to_string(c).ok(),
            Self::Dns(c) => serde_json::to_string(c).ok(),
            Self::Tls(c) => serde_json::to_string(c).ok(),
            Self::Fritzbox(c) => serde_json::to_string(c).ok(),
            Self::Huawei(c) => serde_json::to_string(c).ok(),
            _ => None,
        }
    }

    /// The ping, service or router sensor of `kind` with the stored `params`; `None` if they do
    /// not fit.
    pub fn from_params(kind: &str, params: &str) -> Option<Self> {
        Some(match kind {
            "ping" => Self::Ping(serde_json::from_str(params).ok()?),
            "http" => Self::Http(serde_json::from_str(params).ok()?),
            "tcp" => Self::Tcp(serde_json::from_str(params).ok()?),
            "dns" => Self::Dns(serde_json::from_str(params).ok()?),
            "tls" => Self::Tls(serde_json::from_str(params).ok()?),
            "fritzbox" => Self::Fritzbox(serde_json::from_str(params).ok()?),
            "huawei" => Self::Huawei(serde_json::from_str(params).ok()?),
            _ => return None,
        })
    }

    /// Thresholds a new sensor starts with for channel `key`.
    pub fn default_thresholds(&self, key: &str) -> Thresholds {
        match (self, key) {
            (Self::Tls(_), "days") => Thresholds {
                warn_below: Some(TLS_WARN_DAYS),
                down_below: Some(TLS_DOWN_DAYS),
                ..Thresholds::default()
            },
            (Self::Ping(_), "loss") => Thresholds {
                warn_above: Some(PING_WARN_LOSS),
                ..Thresholds::default()
            },
            (Self::Huawei(_), "signal") => Thresholds {
                warn_below: Some(huawei::WARN_BARS),
                ..Thresholds::default()
            },
            (Self::Huawei(_), "in") => Thresholds {
                warn_above: Some(huawei::WARN_RATE),
                ..Thresholds::default()
            },
            _ => Thresholds::default(),
        }
    }

    /// Short description for lists, e.g. `Interface-Traffic (Index 49153)`.
    pub fn describe(&self, t: Lang) -> String {
        match self {
            Self::Ping(c) => t.get1("ping-describe", "packets", u32::from(c.packets)),
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
            Self::Http(c) => {
                let mut text = c.url.clone();
                if let Some(keyword) = &c.keyword {
                    text.push_str(&t.get1("service-describe-keyword", "keyword", keyword));
                }
                if !c.verify_tls && c.url.starts_with("https:") {
                    text.push_str(&t.get("service-describe-no-verify"));
                }
                text
            }
            Self::Tcp(c) => t.get1("service-describe-tcp", "port", c.port),
            Self::Dns(c) => t.fmt(
                "service-describe-dns",
                &[
                    ("type", c.record.label().into()),
                    ("name", c.name.clone().into()),
                ],
            ),
            Self::Tls(c) => t.get1("service-describe-tls", "port", c.port),
            Self::Fritzbox(c) => t.get(match c.metric {
                FritzMetric::Traffic => "fritzbox-describe-traffic",
                FritzMetric::Connection => "fritzbox-describe-connection",
            }),
            Self::Huawei(c) => t.get(match c.metric {
                HuaweiMetric::Connection => "huawei-describe-connection",
                HuaweiMetric::Traffic => "huawei-describe-traffic",
                HuaweiMetric::Volume => "huawei-describe-volume",
            }),
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
            Self::Ping(_) => [
                ("rtt", Unit::Milliseconds),
                ("rtt_min", Unit::Milliseconds),
                ("rtt_max", Unit::Milliseconds),
                ("jitter", Unit::Milliseconds),
                ("loss", Unit::Percent),
            ]
            .into_iter()
            .map(|(key, unit)| ChannelDef { key, unit })
            .collect(),
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
            Self::Http(_) | Self::Tcp(_) | Self::Dns(_) => one("rtt", Unit::Milliseconds),
            Self::Tls(_) => one("days", Unit::Days),
            Self::Fritzbox(c) => c.channels(),
            Self::Huawei(c) => c.channels(),
        }
    }

    /// Whether channel `key` shows in lists and their small charts. A ping sensor shows only its
    /// average there; minimum, maximum, jitter and loss are on its own page. A router connection
    /// shows what can degrade (sync rates, signal), not the ever rising uptime.
    pub fn in_lists(&self, key: &str) -> bool {
        !matches!(
            (self, key),
            (Self::Ping(_), "rtt_min" | "rtt_max" | "jitter" | "loss")
                | (Self::Fritzbox(_) | Self::Huawei(_), "uptime")
        )
    }

    /// Whether channel `key` gives way on phones in the table of values (ping: minimum, maximum
    /// and jitter; the average and the loss stay).
    pub fn minor(&self, key: &str) -> bool {
        matches!(
            (self, key),
            (Self::Ping(_), "rtt_min" | "rtt_max" | "jitter")
        )
    }

    /// Whether the sensor needs an SNMP session (ping, local and service sensors do not).
    pub fn uses_snmp(&self) -> bool {
        matches!(
            self,
            Self::Uptime
                | Self::InterfaceTraffic { .. }
                | Self::InterfaceStatus { .. }
                | Self::Oid { .. }
        )
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
            Self::Ping(c) => c.validate(),
            Self::Http(c) => c.validate(),
            Self::Tcp(c) => c.validate(),
            Self::Dns(c) => c.validate(),
            Self::Tls(c) => c.validate(),
            Self::Fritzbox(c) => c.validate(),
            Self::Huawei(c) => c.validate(),
            _ => Ok(()),
        }
    }

    /// A fresh sensor instance (counter state starts empty).
    pub fn build(&self) -> Box<dyn Sensor> {
        match self {
            Self::Ping(c) => Box::new(PingSensor::new(*c)),
            Self::Uptime => Box::new(UptimeSensor),
            Self::InterfaceTraffic { if_index } => Box::new(InterfaceTrafficSensor::new(*if_index)),
            Self::InterfaceStatus { if_index } => Box::new(InterfaceStatusSensor::new(*if_index)),
            Self::Oid {
                oid, mode, scale, ..
            } => Box::new(OidSensor::new(oid.clone(), *mode, *scale)),
            Self::Local(metric) => Box::new(LocalSensor::new(metric.clone(), Paths::default())),
            Self::Http(c) => Box::new(HttpSensor::new(c.clone())),
            Self::Tcp(c) => Box::new(TcpSensor::new(c)),
            Self::Dns(c) => Box::new(DnsSensor::new(c.clone())),
            Self::Tls(c) => Box::new(TlsSensor::new(c.clone())),
            Self::Fritzbox(c) => Box::new(FritzSensor::new(*c)),
            Self::Huawei(c) => Box::new(HuaweiSensor::new(*c)),
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
        assert_eq!(
            keys(SensorSpec::ping()),
            ["rtt", "rtt_min", "rtt_max", "jitter", "loss"]
        );
        assert_eq!(
            keys(SensorSpec::InterfaceTraffic { if_index: 1 }),
            ["in", "out"]
        );
        assert_eq!(SensorSpec::Uptime.kind(), "uptime");
        assert!(!SensorSpec::ping().uses_snmp());
        let ping = SensorSpec::ping();
        assert_eq!(
            SensorSpec::from_params("ping", &ping.params().unwrap()),
            Some(ping)
        );
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
            Unit::Days,
        ] {
            assert_eq!(Unit::parse(unit.as_str()), Some(unit));
        }
        assert_eq!(Unit::parse("furlongs"), None);
    }
}
