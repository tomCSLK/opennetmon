//! How sensors are described and their values formatted, using the MIB index where it helps
//! (object names instead of numeric OIDs, enumeration labels, units).

use crate::i18n::Lang;
use crate::mib::{MibIndex, ObjectKind};
use crate::sensors::Unit;
use crate::sensors::format::format_value;
use crate::sensors::snmp::OidMode;
use crate::sensors::spec::SensorSpec;

/// Suggested ratings for the status values of a new sensor, from the names of its MIB enumeration
/// (see [`crate::status::suggest_status`]). Empty when the values have no names or all would be
/// fine anyway, so plain numbers and harmless states stay unrated.
pub fn suggested_states(spec: &SensorSpec, index: &MibIndex) -> Vec<crate::status::ValueState> {
    if !matches!(spec, SensorSpec::Oid { .. }) {
        return Vec::new(); // the interface status sensor has its own default rule
    }
    let states: Vec<crate::status::ValueState> = ValueFormat::for_sensor(spec, index)
        .labels
        .into_iter()
        .map(|(value, label)| crate::status::ValueState {
            status: crate::status::suggest_status(&label),
            value,
            label,
        })
        .collect();
    if states.iter().all(|s| s.status == crate::status::Status::Up) {
        return Vec::new();
    }
    states
}

/// Stores the suggested ratings for a freshly created sensor (see [`suggested_states`]).
pub async fn rate_new_sensor(
    pool: &sqlx::SqlitePool,
    sensor_id: i64,
    spec: &SensorSpec,
    index: &MibIndex,
) -> crate::error::Result<()> {
    let states = suggested_states(spec, index);
    if states.is_empty() {
        return Ok(());
    }
    let channel = crate::storage::sensors::get_sensor(pool, sensor_id)
        .await?
        .and_then(|s| s.channels.first().map(|c| c.id));
    match channel {
        Some(channel) => {
            crate::storage::sensors::set_value_states(pool, sensor_id, channel, &states).await
        }
        None => Ok(()),
    }
}

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

    pub fn format(&self, t: Lang, unit: Unit, value: f64) -> String {
        if value.fract() == 0.0
            && let Some((_, label)) = self.labels.iter().find(|(n, _)| *n as f64 == value)
        {
            return format!("{label} ({value})");
        }
        match (&self.unit_label, unit) {
            (Some(label), Unit::PerSecond) => {
                format!("{} {label}/s", format_value(t, Unit::Count, value))
            }
            (Some(label), _) => format!("{} {label}", format_value(t, unit, value)),
            (None, _) => format_value(t, unit, value),
        }
    }
}

/// [`describe`] plus the sensor's own polling interval, if it has one.
pub fn describe_sensor(
    t: Lang,
    sensor: &crate::storage::sensors::SensorRecord,
    index: &MibIndex,
) -> String {
    let text = describe(t, &sensor.spec, index);
    match sensor.interval_secs {
        Some(secs) => format!("{text} · {}", t.get1("every-seconds", "seconds", secs)),
        None => text,
    }
}

/// Description of a sensor for lists: custom OIDs by their MIB name when known.
pub fn describe(t: Lang, spec: &SensorSpec, index: &MibIndex) -> String {
    match spec {
        SensorSpec::Oid { oid, .. } => {
            let Some((object, _)) = index.describe(oid) else {
                return spec.describe(t);
            };
            if !matches!(object.kind, ObjectKind::Scalar | ObjectKind::Column) {
                return spec.describe(t);
            }
            let name = index.name_of(oid).unwrap_or_default();
            format!("{name} ({}) · {}", object.module, spec.oid_details(t))
        }
        _ => spec.describe(t),
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
            describe(Lang::De, &spec, &index),
            "hrProcessorLoad.196608 (HOST-RESOURCES-MIB) · Wert, Einheit %"
        );
        let unknown = oid_spec(&[1, 3, 6, 1, 4, 1, 99999, 1, 0], OidMode::Gauge, None);
        assert!(describe(Lang::De, &unknown, &index).starts_with("OID 1.3.6.1.4.1.99999.1.0"));
    }

    #[test]
    fn values_get_enumeration_labels_and_units() {
        let index = bundled_index();
        let status = ValueFormat::for_sensor(&SensorSpec::InterfaceStatus { if_index: 1 }, &index);
        assert_eq!(status.format(Lang::De, Unit::State, 1.0), "up (1)");
        assert_eq!(
            status.format(Lang::De, Unit::State, 7.0),
            "lowerLayerDown (7)"
        );
        assert_eq!(status.format(Lang::De, Unit::State, 42.0), "42");

        let oper = oid_spec(&[1, 3, 6, 1, 2, 1, 2, 2, 1, 8, 3], OidMode::Gauge, None);
        assert_eq!(
            ValueFormat::for_sensor(&oper, &index).format(Lang::De, Unit::Count, 2.0),
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
        assert_eq!(temp.format(Lang::De, Unit::Count, 41.5), "41,5 °C");
        assert_eq!(temp.format(Lang::De, Unit::PerSecond, 3.0), "3 °C/s");
        assert_eq!(
            ValueFormat::default().format(Lang::De, Unit::Count, 2.0),
            "2"
        );
    }
}
