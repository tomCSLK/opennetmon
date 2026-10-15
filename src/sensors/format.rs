//! Human-readable values and labels (German: decimal comma).

use super::Unit;

/// Number with at most `decimals` decimals, no trailing zeros, decimal comma.
fn num(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    text.replace('.', ",")
}

/// A channel value with a suitable unit prefix, e.g. `12,5 Mbit/s` or `3 Tage 4 Std. 5 Min.`.
pub fn format_value(unit: Unit, value: f64) -> String {
    match unit {
        Unit::BitsPerSecond => {
            let abs = value.abs();
            if abs >= 1e9 {
                format!("{} Gbit/s", num(value / 1e9, 2))
            } else if abs >= 1e6 {
                format!("{} Mbit/s", num(value / 1e6, 2))
            } else if abs >= 1e3 {
                format!("{} kbit/s", num(value / 1e3, 1))
            } else {
                format!("{} bit/s", num(value, 0))
            }
        }
        Unit::Milliseconds => format!("{} ms", num(value, 2)),
        Unit::Seconds => {
            let total = value.max(0.0) as u64;
            let (days, hours, minutes) = (
                total / 86_400,
                (total % 86_400) / 3_600,
                (total % 3_600) / 60,
            );
            match (days, hours, minutes) {
                (0, 0, 0) => format!("{total} s"),
                (0, 0, m) => format!("{m} Min. {} s", total % 60),
                (0, h, m) => format!("{h} Std. {m} Min."),
                (d, h, m) => format!("{d} Tage {h} Std. {m} Min."),
            }
        }
        Unit::Percent => format!("{} %", num(value, 1)),
        Unit::Count => num(value, 3),
        Unit::PerSecond => format!("{} /s", num(value, 3)),
        Unit::State => num(value, 0),
    }
}

/// Unit shown next to threshold inputs; thresholds are entered in this unit.
pub fn input_unit(unit: Unit) -> &'static str {
    match unit {
        Unit::BitsPerSecond => "Mbit/s",
        Unit::Milliseconds => "ms",
        Unit::Seconds => "s",
        Unit::Percent => "%",
        Unit::PerSecond => "/s",
        Unit::Count | Unit::State => "",
    }
}

/// Factor from the input unit ([`input_unit`]) to the stored base unit.
pub fn input_factor(unit: Unit) -> f64 {
    match unit {
        Unit::BitsPerSecond => 1e6,
        _ => 1.0,
    }
}

/// German label of a channel key.
pub fn channel_label(key: &str) -> &str {
    match key {
        "in" => "Eingehend",
        "out" => "Ausgehend",
        "rtt" => "Antwortzeit",
        "uptime" => "Laufzeit",
        "oper_status" => "Status",
        "value" => "Wert",
        other => other,
    }
}

/// A threshold typed by the user (decimal comma allowed) in the input unit, as a base-unit value.
/// Empty input means "no threshold".
pub fn parse_threshold(text: &str, unit: Unit) -> Result<Option<f64>, &'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let value: f64 = text
        .replace(',', ".")
        .parse()
        .map_err(|_| "Ein Schwellwert muss eine Zahl sein.")?;
    if !value.is_finite() {
        return Err("Ein Schwellwert muss eine endliche Zahl sein.");
    }
    Ok(Some(value * input_factor(unit)))
}

/// Inverse of [`parse_threshold`] for prefilling the form.
pub fn threshold_input(value: Option<f64>, unit: Unit) -> String {
    value.map_or_else(String::new, |v| num(v / input_factor(unit), 6))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_rates_get_a_prefix() {
        assert_eq!(format_value(Unit::BitsPerSecond, 0.0), "0 bit/s");
        assert_eq!(format_value(Unit::BitsPerSecond, 999.4), "999 bit/s");
        assert_eq!(format_value(Unit::BitsPerSecond, 3815.38), "3,8 kbit/s");
        assert_eq!(
            format_value(Unit::BitsPerSecond, 12_500_000.0),
            "12,5 Mbit/s"
        );
        assert_eq!(
            format_value(Unit::BitsPerSecond, 1_000_000_000.0),
            "1 Gbit/s"
        );
    }

    #[test]
    fn other_units() {
        assert_eq!(format_value(Unit::Milliseconds, 1.97298), "1,97 ms");
        assert_eq!(format_value(Unit::Percent, 12.34), "12,3 %");
        assert_eq!(format_value(Unit::Count, 21.5), "21,5");
        assert_eq!(format_value(Unit::PerSecond, 100.0), "100 /s");
        assert_eq!(format_value(Unit::State, 1.0), "1");
        assert_eq!(format_value(Unit::Seconds, 45.0), "45 s");
        assert_eq!(format_value(Unit::Seconds, 125.0), "2 Min. 5 s");
        assert_eq!(format_value(Unit::Seconds, 7_380.0), "2 Std. 3 Min.");
        assert_eq!(
            format_value(Unit::Seconds, 16_191_958.64),
            "187 Tage 9 Std. 45 Min."
        );
    }

    #[test]
    fn thresholds_round_trip_in_the_input_unit() {
        assert_eq!(
            parse_threshold(" 12,5 ", Unit::BitsPerSecond),
            Ok(Some(12_500_000.0))
        );
        assert_eq!(parse_threshold("", Unit::Milliseconds), Ok(None));
        assert_eq!(parse_threshold("200", Unit::Milliseconds), Ok(Some(200.0)));
        assert!(parse_threshold("abc", Unit::Milliseconds).is_err());
        assert!(parse_threshold("inf", Unit::Milliseconds).is_err());
        assert!(parse_threshold("NaN", Unit::Milliseconds).is_err());
        assert_eq!(
            threshold_input(Some(12_500_000.0), Unit::BitsPerSecond),
            "12,5"
        );
        assert_eq!(threshold_input(None, Unit::Seconds), "");
        assert_eq!(threshold_input(Some(200.0), Unit::Milliseconds), "200");
    }

    #[test]
    fn channel_labels_are_german() {
        assert_eq!(channel_label("in"), "Eingehend");
        assert_eq!(channel_label("custom"), "custom");
    }
}
