//! How sensors are described and their values formatted, using the MIB index where it helps
//! (object names instead of numeric OIDs, enumeration labels, units).

use crate::mib::{MibIndex, ObjectKind};
use crate::sensors::Unit;
use crate::sensors::format::format_value;
use crate::sensors::snmp::OidMode;
use crate::sensors::spec::SensorSpec;

fn swap(enums: &[(String, i64)]) -> Vec<(i64, String)> {
    enums.iter().map(|(name, n)| (*n, name.clone())).collect()
}

/// Display details beyond the channel's unit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValueFormat {
    pub unit_label: Option<String>,
    /// Enumeration labels from the MIB (e.g. 1 = up).
    pub labels: Vec<(i64, String)>,
}

impl ValueFormat {
    pub fn for_sensor(spec: &SensorSpec, index: &MibIndex) -> Self {
        match spec {
            SensorSpec::Oid {
                oid, unit, mode, ..
            } => {
                let object = index
                    .describe(oid)
                    .map(|(o, _)| o)
                    .filter(|o| matches!(o.kind, ObjectKind::Scalar | ObjectKind::Column));
                Self {
                    unit_label: unit.clone(),
                    // A rate has no enumeration meaning.
                    labels: match mode {
                        OidMode::Gauge => object.map(|o| swap(&o.enums)).unwrap_or_default(),
                        OidMode::Counter => Vec::new(),
                    },
                }
            }
            SensorSpec::InterfaceStatus { .. } => Self {
                unit_label: None,
                labels: index
                    .by_name("ifOperStatus", Some("IF-MIB"))
                    .first()
                    .map(|o| swap(&o.enums))
                    .unwrap_or_default(),
            },
            _ => Self::default(),
        }
    }

    pub fn format(&self, unit: Unit, value: f64) -> String {
        if value.fract() == 0.0
            && let Some((_, label)) = self.labels.iter().find(|(n, _)| *n as f64 == value)
        {
            return format!("{label} ({value})");
        }
        match (&self.unit_label, unit) {
            (Some(label), Unit::PerSecond) => {
                format!("{} {label}/s", format_value(Unit::Count, value))
            }
            (Some(label), _) => format!("{} {label}", format_value(unit, value)),
            (None, _) => format_value(unit, value),
        }
    }
}

/// Description of a sensor for lists: custom OIDs by their MIB name when known.
pub fn describe(spec: &SensorSpec, index: &MibIndex) -> String {
    match spec {
        SensorSpec::Oid {
            oid,
            mode,
            scale,
            unit,
        } => {
            let Some((object, _)) = index.describe(oid) else {
                return spec.describe();
            };
            if !matches!(object.kind, ObjectKind::Scalar | ObjectKind::Column) {
                return spec.describe();
            }
            let name = index.name_of(oid).unwrap_or_default();
            let mut text = format!(
                "{name} ({}) · {}",
                object.module,
                match mode {
                    OidMode::Gauge => "Wert",
                    OidMode::Counter => "Zähler, Rate/s",
                }
            );
            if *scale != 1.0 {
                text.push_str(&format!(", Faktor {scale}"));
            }
            if let Some(u) = unit {
                text.push_str(&format!(", Einheit {u}"));
            }
            text
        }
        _ => spec.describe(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mib::bundled_index;

    fn oid_spec(oid: &[u64], mode: OidMode, unit: Option<&str>) -> SensorSpec {
        SensorSpec::Oid {
            oid: oid.to_vec(),
            mode,
            scale: 1.0,
            unit: unit.map(str::to_owned),
        }
    }

    #[test]
    fn known_oids_are_described_by_name() {
        let index = bundled_index();
        let spec = oid_spec(
            &[1, 3, 6, 1, 2, 1, 25, 3, 3, 1, 2, 196608],
            OidMode::Gauge,
            Some("%"),
        );
        assert_eq!(
            describe(&spec, &index),
            "hrProcessorLoad.196608 (HOST-RESOURCES-MIB) · Wert, Einheit %"
        );
        let unknown = oid_spec(&[1, 3, 6, 1, 4, 1, 99999, 1, 0], OidMode::Gauge, None);
        assert!(describe(&unknown, &index).starts_with("OID 1.3.6.1.4.1.99999.1.0"));
    }

    #[test]
    fn values_get_enumeration_labels_and_units() {
        let index = bundled_index();
        let status = ValueFormat::for_sensor(&SensorSpec::InterfaceStatus { if_index: 1 }, &index);
        assert_eq!(status.format(Unit::State, 1.0), "up (1)");
        assert_eq!(status.format(Unit::State, 7.0), "lowerLayerDown (7)");
        assert_eq!(status.format(Unit::State, 42.0), "42");

        let oper = oid_spec(&[1, 3, 6, 1, 2, 1, 2, 2, 1, 8, 3], OidMode::Gauge, None);
        assert_eq!(
            ValueFormat::for_sensor(&oper, &index).format(Unit::Count, 2.0),
            "down (2)"
        );
        let rate = oid_spec(&[1, 3, 6, 1, 2, 1, 2, 2, 1, 8, 3], OidMode::Counter, None);
        assert!(
            ValueFormat::for_sensor(&rate, &index).labels.is_empty(),
            "rates have no labels"
        );

        let temp = ValueFormat {
            unit_label: Some("°C".to_owned()),
            labels: Vec::new(),
        };
        assert_eq!(temp.format(Unit::Count, 41.5), "41,5 °C");
        assert_eq!(temp.format(Unit::PerSecond, 3.0), "3 °C/s");
        assert_eq!(ValueFormat::default().format(Unit::Count, 2.0), "2");
    }
}
