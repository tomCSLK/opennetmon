//! Human-readable values and labels in the reader's language (German: decimal comma).

use super::Unit;
use crate::i18n::{Lang, Msg};

/// Number with at most `decimals` decimals, no trailing zeros, with the language's decimal separator.
fn num(t: Lang, value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    text.replace('.', &t.decimal_separator().to_string())
}

/// A channel value with a suitable unit prefix, e.g. `12,5 Mbit/s` or `3 Tage 4 Std. 5 Min.`.
pub fn format_value(t: Lang, unit: Unit, value: f64) -> String {
    match unit {
        Unit::BitsPerSecond => {
            let abs = value.abs();
            if abs >= 1e9 {
                format!("{} Gbit/s", num(t, value / 1e9, 2))
            } else if abs >= 1e6 {
                format!("{} Mbit/s", num(t, value / 1e6, 2))
            } else if abs >= 1e3 {
                format!("{} kbit/s", num(t, value / 1e3, 1))
            } else {
                format!("{} bit/s", num(t, value, 0))
            }
        }
        Unit::Milliseconds => format!("{} ms", num(t, value, 2)),
        Unit::Seconds => {
            let total = value.max(0.0) as u64;
            let (days, hours, minutes) = (
                total / 86_400,
                (total % 86_400) / 3_600,
                (total % 3_600) / 60,
            );
            match (days, hours, minutes) {
                (0, 0, 0) => format!("{total} s"),
                (0, 0, m) => t.get2(
                    "duration-minutes-seconds",
                    "minutes",
                    m,
                    "seconds",
                    total % 60,
                ),
                (0, h, m) => t.get2("duration-hours-minutes", "hours", h, "minutes", m),
                (d, h, m) => t.fmt(
                    "duration-days-hours-minutes",
                    &[
                        ("days", d.into()),
                        ("hours", h.into()),
                        ("minutes", m.into()),
                    ],
                ),
            }
        }
        Unit::Percent => format!("{} %", num(t, value, 1)),
        Unit::Count => num(t, value, 3),
        Unit::PerSecond => format!("{} /s", num(t, value, 3)),
        Unit::State => num(t, value, 0),
        Unit::Bytes => bytes(t, value),
        Unit::Celsius => format!("{} °C", num(t, value, 1)),
        Unit::Days => t.get1("value-days", "days", value.floor() as i64),
    }
}

/// Data volume in decimal units, as the charts show it (1 GB = 1 000 000 000 bytes).
fn bytes(t: Lang, value: f64) -> String {
    let abs = value.abs();
    for (factor, unit) in [(1e12, "TB"), (1e9, "GB"), (1e6, "MB")] {
        if abs >= factor {
            return format!("{} {unit}", num(t, value / factor, 2));
        }
    }
    if abs >= 1e3 {
        format!("{} KB", num(t, value / 1e3, 1))
    } else {
        format!("{} B", num(t, value, 0))
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
        Unit::Bytes => "GB",
        Unit::Celsius => "°C",
        Unit::Days => "d",
        Unit::Count | Unit::State => "",
    }
}

/// Factor from the input unit ([`input_unit`]) to the stored base unit.
pub fn input_factor(unit: Unit) -> f64 {
    match unit {
        Unit::BitsPerSecond => 1e6,
        Unit::Bytes => 1e9,
        _ => 1.0,
    }
}

/// Label of a channel key: the built-in keys are translated, others (e.g. from MIB tables) shown as they are.
pub fn channel_label(key: &str) -> Msg {
    match key {
        "in" => Msg::new("channel-in"),
        "out" => Msg::new("channel-out"),
        "rtt" => Msg::new("channel-rtt"),
        "uptime" => Msg::new("channel-uptime"),
        "oper_status" => Msg::new("channel-oper-status"),
        "value" => Msg::new("channel-value"),
        "total" => Msg::new("channel-total"),
        "iowait" => Msg::new("channel-iowait"),
        "load1" => Msg::new("channel-load1"),
        "load5" => Msg::new("channel-load5"),
        "load15" => Msg::new("channel-load15"),
        "used" => Msg::new("channel-used"),
        "available" => Msg::new("channel-available"),
        "swap_used" => Msg::new("channel-swap-used"),
        "free" => Msg::new("channel-free"),
        "temperature" => Msg::new("channel-temperature"),
        "days" => Msg::new("channel-days"),
        other => Msg::Text(other.to_owned()),
    }
}

/// A threshold typed by the user (decimal comma allowed) in the input unit, as a base-unit value.
/// Empty input means "no threshold".
pub fn parse_threshold(text: &str, unit: Unit) -> Result<Option<f64>, Msg> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let value: f64 = text
        .replace(',', ".")
        .parse()
        .map_err(|_| Msg::new("threshold-not-a-number"))?;
    if !value.is_finite() {
        return Err(Msg::new("threshold-not-finite"));
    }
    Ok(Some(value * input_factor(unit)))
}

/// Inverse of [`parse_threshold`] for prefilling the form.
pub fn threshold_input(t: Lang, value: Option<f64>, unit: Unit) -> String {
    value.map_or_else(String::new, |v| num(t, v / input_factor(unit), 6))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_rates_get_a_prefix() {
        assert_eq!(format_value(Lang::De, Unit::BitsPerSecond, 0.0), "0 bit/s");
        assert_eq!(
            format_value(Lang::De, Unit::BitsPerSecond, 999.4),
            "999 bit/s"
        );
        assert_eq!(
            format_value(Lang::De, Unit::BitsPerSecond, 3815.38),
            "3,8 kbit/s"
        );
        assert_eq!(
            format_value(Lang::De, Unit::BitsPerSecond, 12_500_000.0),
            "12,5 Mbit/s"
        );
        assert_eq!(
            format_value(Lang::De, Unit::BitsPerSecond, 1_000_000_000.0),
            "1 Gbit/s"
        );
    }

    #[test]
    fn other_units() {
        assert_eq!(
            format_value(Lang::De, Unit::Milliseconds, 1.97298),
            "1,97 ms"
        );
        assert_eq!(format_value(Lang::De, Unit::Percent, 12.34), "12,3 %");
        assert_eq!(format_value(Lang::De, Unit::Count, 21.5), "21,5");
        assert_eq!(format_value(Lang::De, Unit::PerSecond, 100.0), "100 /s");
        assert_eq!(format_value(Lang::De, Unit::State, 1.0), "1");
        assert_eq!(format_value(Lang::De, Unit::Celsius, 45.25), "45,2 °C");
        assert_eq!(format_value(Lang::De, Unit::Days, 13.6), "13 Tage");
        assert_eq!(format_value(Lang::En, Unit::Days, 1.2), "1 day");
        assert_eq!(format_value(Lang::En, Unit::Days, -1.5), "-2 days");
        assert_eq!(format_value(Lang::De, Unit::Bytes, 512.0), "512 B");
        assert_eq!(format_value(Lang::De, Unit::Bytes, 12_345.0), "12,3 KB");
        assert_eq!(
            format_value(Lang::De, Unit::Bytes, 250_000_000_000.0),
            "250 GB"
        );
        assert_eq!(parse_threshold("20", Unit::Bytes), Ok(Some(20e9)));
        assert_eq!(format_value(Lang::De, Unit::Seconds, 45.0), "45 s");
        assert_eq!(format_value(Lang::De, Unit::Seconds, 125.0), "2 Min. 5 s");
        assert_eq!(
            format_value(Lang::De, Unit::Seconds, 7_380.0),
            "2 Std. 3 Min."
        );
        assert_eq!(
            format_value(Lang::De, Unit::Seconds, 16_191_958.64),
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
            threshold_input(Lang::De, Some(12_500_000.0), Unit::BitsPerSecond),
            "12,5"
        );
        assert_eq!(threshold_input(Lang::De, None, Unit::Seconds), "");
        assert_eq!(
            threshold_input(Lang::De, Some(200.0), Unit::Milliseconds),
            "200"
        );
    }

    #[test]
    fn channel_labels_are_translated_unless_custom() {
        assert_eq!(Lang::De.msg(&channel_label("in")), "Eingehend");
        assert_eq!(Lang::En.msg(&channel_label("rtt")), "Response time");
        assert_eq!(Lang::En.msg(&channel_label("custom")), "custom");
    }

    #[test]
    fn english_values_use_a_decimal_point() {
        assert_eq!(
            format_value(Lang::En, Unit::BitsPerSecond, 12_500_000.0),
            "12.5 Mbit/s"
        );
        assert_eq!(format_value(Lang::En, Unit::Seconds, 125.0), "2 min 5 s");
        assert_eq!(
            format_value(Lang::En, Unit::Seconds, 16_191_958.64),
            "187 days 9 h 45 min"
        );
        assert_eq!(
            threshold_input(Lang::En, Some(12_500_000.0), Unit::BitsPerSecond),
            "12.5"
        );
    }
}
