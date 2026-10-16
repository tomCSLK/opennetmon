//! Known tables and scalar groups with measured values, with German labels, factors and units, plus
//! a heuristic that reads units like "in 0.1W" from descriptions (many vendor MIBs have no UNITS).

/// One value column (or scalar) of a catalog entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnSpec {
    pub object: &'static str,
    pub label: &'static str,
    pub scale: f64,
    pub unit: Option<&'static str>,
    /// Ticked by default on the discovery page.
    pub selected: bool,
}

/// How rows of a table are named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowLabel {
    /// `{i}` = index, `{n}` = position (1, 2, …).
    Pattern(&'static str),
    /// A text column of the same table.
    Column(&'static str),
    /// `entPhysicalName` of the ENTITY-MIB for `entPhysicalIndex`.
    Physical,
}

/// Rows whose factor and unit depend on other columns of the same row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowRule {
    Plain,
    /// ENTITY-SENSOR-MIB: type, scale and precision columns decide factor and unit.
    EntitySensor,
    /// HOST-RESOURCES-MIB storage: sizes are in units of `hrStorageAllocationUnits`.
    HrStorage,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CatalogEntry {
    pub key: &'static str,
    pub title: &'static str,
    pub module: &'static str,
    /// A table (`…Table`) or an OBJECT IDENTIFIER whose children are scalars.
    pub object: &'static str,
    pub columns: &'static [ColumnSpec],
    pub rows: RowLabel,
    pub rule: RowRule,
}

const fn col(
    object: &'static str,
    label: &'static str,
    scale: f64,
    unit: Option<&'static str>,
    selected: bool,
) -> ColumnSpec {
    ColumnSpec {
        object,
        label,
        scale,
        unit,
        selected,
    }
}

pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        key: "tp-poe-ports",
        title: "PoE je Port (TP-Link)",
        module: "TPLINK-POWER-OVER-ETHERNET-MIB",
        object: "tpPoePortConfigTable",
        columns: &[
            col("tpPoePower", "PoE-Leistung", 0.1, Some("W"), true),
            col("tpPoePowerStatus", "PoE-Status", 1.0, None, true),
            col("tpPoeCurrent", "PoE-Strom", 1.0, Some("mA"), false),
            col("tpPoeVoltage", "PoE-Spannung", 0.1, Some("V"), false),
            col("tpPoeClass", "PoE-Klasse", 1.0, None, false),
        ],
        rows: RowLabel::Pattern("Port {i}"),
        rule: RowRule::Plain,
    },
    CatalogEntry {
        key: "tp-poe-total",
        title: "PoE gesamt (TP-Link)",
        module: "TPLINK-POWER-OVER-ETHERNET-MIB",
        object: "tpPoeGlobal",
        columns: &[
            col(
                "tpSystemPowerConsumption",
                "PoE-Verbrauch gesamt",
                0.1,
                Some("W"),
                true,
            ),
            col("tpSystemPowerRemain", "PoE-Reserve", 0.1, Some("W"), false),
            col("tpSystemPowerLimit", "PoE-Budget", 0.1, Some("W"), false),
        ],
        rows: RowLabel::Pattern(""),
        rule: RowRule::Plain,
    },
    CatalogEntry {
        key: "tp-cpu",
        title: "CPU-Auslastung (TP-Link)",
        module: "TPLINK-SYSMONITOR-MIB",
        object: "tpSysMonitorCpuTable",
        columns: &[
            col("tpSysMonitorCpu1Minute", "CPU 1 Min.", 1.0, Some("%"), true),
            col("tpSysMonitorCpu5Seconds", "CPU 5 s", 1.0, Some("%"), false),
            col(
                "tpSysMonitorCpu5Minutes",
                "CPU 5 Min.",
                1.0,
                Some("%"),
                false,
            ),
        ],
        rows: RowLabel::Pattern("Unit {i}"),
        rule: RowRule::Plain,
    },
    CatalogEntry {
        key: "tp-memory",
        title: "Speicher-Auslastung (TP-Link)",
        module: "TPLINK-SYSMONITOR-MIB",
        object: "tpSysMonitorMemoryTable",
        columns: &[col(
            "tpSysMonitorMemoryUtilization",
            "Speicher",
            1.0,
            Some("%"),
            true,
        )],
        rows: RowLabel::Pattern("Unit {i}"),
        rule: RowRule::Plain,
    },
    CatalogEntry {
        key: "hr-cpu",
        title: "CPU-Last (HOST-RESOURCES-MIB)",
        module: "HOST-RESOURCES-MIB",
        object: "hrProcessorTable",
        columns: &[col("hrProcessorLoad", "CPU-Last", 1.0, Some("%"), true)],
        rows: RowLabel::Pattern("CPU {n}"),
        rule: RowRule::Plain,
    },
    CatalogEntry {
        key: "hr-storage",
        title: "Speicher und Datenträger (HOST-RESOURCES-MIB)",
        module: "HOST-RESOURCES-MIB",
        object: "hrStorageTable",
        columns: &[
            col("hrStorageUsed", "Belegt", 1.0, Some("MB"), true),
            col("hrStorageSize", "Größe", 1.0, Some("MB"), false),
        ],
        rows: RowLabel::Column("hrStorageDescr"),
        rule: RowRule::HrStorage,
    },
    CatalogEntry {
        key: "ent-sensors",
        title: "Hardware-Sensoren: Temperatur, Lüfter, Spannung (ENTITY-SENSOR-MIB)",
        module: "ENTITY-SENSOR-MIB",
        object: "entPhySensorTable",
        columns: &[
            col("entPhySensorValue", "Messwert", 1.0, None, true),
            col("entPhySensorOperStatus", "Sensorstatus", 1.0, None, false),
        ],
        rows: RowLabel::Physical,
        rule: RowRule::EntitySensor,
    },
];

/// Factor and unit of an ENTITY-SENSOR-MIB value from its type, scale and precision.
pub fn entity_sensor_unit(
    sensor_type: i64,
    scale: i64,
    precision: i64,
) -> (f64, Option<&'static str>) {
    let unit = match sensor_type {
        3 => Some("V AC"),
        4 => Some("V"),
        5 => Some("A"),
        6 => Some("W"),
        7 => Some("Hz"),
        8 => Some("°C"),
        9 => Some("% rF"),
        10 => Some("U/min"),
        11 => Some("m³/min"),
        _ => None,
    };
    let exponent = 3 * (scale.clamp(1, 17) - 9) - precision.clamp(-8, 9);
    (10f64.powi(i32::try_from(exponent).unwrap_or(0)), unit)
}

/// Units a description may name, longest spelling first: `(spelling, unit label, needs a number)`.
const UNIT_WORDS: &[(&str, &str, bool)] = &[
    ("celsius", "°C", false),
    ("degrees", "°C", false),
    ("percent", "%", false),
    ("watts", "W", false),
    ("watt", "W", false),
    ("volts", "V", false),
    ("volt", "V", false),
    ("amperes", "A", false),
    ("ampere", "A", false),
    ("mbps", "Mbit/s", false),
    ("kbps", "kbit/s", false),
    ("rpm", "U/min", false),
    ("dbm", "dBm", false),
    ("°c", "°C", false),
    ("mw", "mW", true),
    ("mv", "mV", true),
    ("ma", "mA", true),
    ("%", "%", false),
    ("w", "W", true),
    ("v", "V", true),
    ("a", "A", true),
    ("c", "°C", true),
];

/// Reads "in 0.1W", "unit is 0.1W", "in 1mA", "in percent", "in Celsius" from a description:
/// `(factor, unit)`. Only after "in", "is" or "of", so "in 5 seconds" or "a power of" do not count.
pub fn unit_from_description(description: &str) -> Option<(f64, String)> {
    let text = description.to_lowercase();
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '"'))
        .filter(|w| !w.is_empty())
        .collect();
    for (i, word) in words.iter().enumerate() {
        if !matches!(*word, "in" | "is" | "of" | "unit:" | "units:") {
            continue;
        }
        let Some(next) = words.get(i + 1) else {
            continue;
        };
        let next = next.trim_end_matches('.');
        // A number, glued to the unit ("0.1w") or followed by it ("0.1 w").
        let digits: String = next
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let (factor, unit_text) = if digits.is_empty() {
            (None, next.to_owned())
        } else {
            let rest = &next[digits.len()..];
            let unit_text = if rest.is_empty() {
                words
                    .get(i + 2)
                    .map(|w| w.trim_end_matches('.').to_owned())
                    .unwrap_or_default()
            } else {
                rest.to_owned()
            };
            (digits.trim_end_matches('.').parse::<f64>().ok(), unit_text)
        };
        if let Some((_, label, needs_number)) = UNIT_WORDS
            .iter()
            .find(|(spelling, _, _)| unit_text == *spelling)
            && (factor.is_some() || !needs_number)
        {
            let factor = factor.filter(|f| f.is_finite() && *f > 0.0).unwrap_or(1.0);
            return Some((factor, (*label).to_owned()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_are_read_from_vendor_descriptions() {
        assert_eq!(
            unit_from_description(" Displays the port's real time power supply in 0.1W."),
            Some((0.1, "W".to_owned()))
        );
        assert_eq!(
            unit_from_description(" Displays the port's real time current in 1mA."),
            Some((1.0, "mA".to_owned()))
        );
        assert_eq!(
            unit_from_description(" Displays the port's real time voltage in 0.1V."),
            Some((0.1, "V".to_owned()))
        );
        assert_eq!(
            unit_from_description("Real time consumption. The unit is 0.1W."),
            Some((0.1, "W".to_owned()))
        );
        assert_eq!(
            unit_from_description("The temperature in Celsius."),
            Some((1.0, "°C".to_owned()))
        );
        assert_eq!(
            unit_from_description("Utilization in percent"),
            Some((1.0, "%".to_owned()))
        );
        assert_eq!(
            unit_from_description("Speed in 0.5 W steps"),
            Some((0.5, "W".to_owned()))
        );
    }

    #[test]
    fn ordinary_sentences_are_not_mistaken_for_units() {
        for text in [
            "Displays the CPU utilization in 5 seconds.",
            "Displays the memory utilization.",
            "A value of 1 means enabled.",
            "in a table",
            "is a counter",
            "",
        ] {
            assert_eq!(unit_from_description(text), None, "{text}");
        }
    }

    #[test]
    fn entity_sensors_get_factor_and_unit_from_scale_and_precision() {
        assert_eq!(
            entity_sensor_unit(8, 9, 1),
            (0.1, Some("°C")),
            "degrees C * 10"
        );
        assert_eq!(
            entity_sensor_unit(4, 8, 0),
            (0.001, Some("V")),
            "millivolts"
        );
        assert_eq!(entity_sensor_unit(10, 9, 0), (1.0, Some("U/min")));
        assert_eq!(entity_sensor_unit(1, 9, 0), (1.0, None));
    }

    #[test]
    fn catalog_entries_have_columns_and_unique_keys() {
        let mut keys: Vec<_> = CATALOG.iter().map(|e| e.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), CATALOG.len());
        assert!(CATALOG.iter().all(|e| !e.columns.is_empty()));
    }
}
