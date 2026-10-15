//! Counter-to-rate conversion with wrap and reset detection.
//!
//! A rate is only produced when it is trustworthy. A counter that went backwards without a
//! plausible wrap (device reboot, counter reset) yields [`Rate::Skipped`] instead of an outlier.

/// Width of an SNMP counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CounterWidth {
    Bits32,
    Bits64,
}

impl CounterWidth {
    fn max_value(self) -> u64 {
        match self {
            Self::Bits32 => u64::from(u32::MAX),
            Self::Bits64 => u64::MAX,
        }
    }
}

/// One observation of a counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CounterReading {
    pub value: u64,
    /// Poll time, milliseconds since the Unix epoch (UTC).
    pub at_ms: i64,
    /// `sysUpTime` of the device in timeticks (1/100 s) taken in the same request, if known.
    pub uptime_ticks: Option<u32>,
}

/// Why no rate was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The clock did not advance between the two readings.
    NonMonotonicTime,
    /// The value does not fit the counter width.
    InvalidValue,
    /// `sysUpTime` decreased: the device rebooted (or the uptime itself wrapped).
    DeviceRestarted,
    /// The counter decreased and a wrap cannot be confirmed.
    CounterReset,
    /// The rate exceeds the configured plausible maximum.
    ImplausibleRate,
}

/// Result of feeding a reading into a [`CounterTracker`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rate {
    /// First reading (or first after a skip): nothing to compare with yet.
    First,
    /// Units per second.
    PerSecond(f64),
    Skipped(SkipReason),
}

/// Remembers the previous reading of one counter and derives rates.
#[derive(Debug, Clone)]
pub struct CounterTracker {
    width: CounterWidth,
    /// Largest rate (units/s) considered plausible, e.g. link speed in bytes/s.
    max_rate: Option<f64>,
    previous: Option<CounterReading>,
}

impl CounterTracker {
    pub fn new(width: CounterWidth, max_rate: Option<f64>) -> Self {
        Self {
            width,
            max_rate,
            previous: None,
        }
    }

    /// Computes the rate since the previous reading. The new reading always becomes the
    /// baseline, so a reset costs exactly one sample.
    pub fn update(&mut self, current: CounterReading) -> Rate {
        let previous = self.previous.replace(current);
        let Some(previous) = previous else {
            return Rate::First;
        };
        self.rate(previous, current)
    }

    fn rate(&self, prev: CounterReading, cur: CounterReading) -> Rate {
        let max = self.width.max_value();
        if prev.value > max || cur.value > max {
            return Rate::Skipped(SkipReason::InvalidValue);
        }
        let elapsed_ms = cur.at_ms.saturating_sub(prev.at_ms);
        if elapsed_ms <= 0 {
            return Rate::Skipped(SkipReason::NonMonotonicTime);
        }
        let uptime_confirms = match (prev.uptime_ticks, cur.uptime_ticks) {
            (Some(p), Some(c)) if c < p => return Rate::Skipped(SkipReason::DeviceRestarted),
            (Some(_), Some(_)) => true,
            _ => false,
        };

        let delta = if cur.value >= prev.value {
            cur.value - prev.value
        } else {
            // A decrease is only a wrap for 32-bit counters (a 64-bit wrap takes centuries),
            // and only if the device did not restart or the rate stays plausible.
            if self.width == CounterWidth::Bits64 {
                return Rate::Skipped(SkipReason::CounterReset);
            }
            if !uptime_confirms && self.max_rate.is_none() {
                return Rate::Skipped(SkipReason::CounterReset);
            }
            max - prev.value + cur.value + 1
        };

        let rate = delta as f64 / (elapsed_ms as f64 / 1000.0);
        if self.max_rate.is_some_and(|limit| rate > limit) {
            // A bogus wrap shows up as a huge rate; so does a counter jump.
            return Rate::Skipped(if cur.value < prev.value {
                SkipReason::CounterReset
            } else {
                SkipReason::ImplausibleRate
            });
        }
        Rate::PerSecond(rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(value: u64, at_ms: i64, uptime: Option<u32>) -> CounterReading {
        CounterReading {
            value,
            at_ms,
            uptime_ticks: uptime,
        }
    }

    fn two(
        width: CounterWidth,
        max_rate: Option<f64>,
        a: CounterReading,
        b: CounterReading,
    ) -> Rate {
        let mut t = CounterTracker::new(width, max_rate);
        assert_eq!(t.update(a), Rate::First);
        t.update(b)
    }

    #[test]
    fn first_reading_has_no_rate() {
        let mut t = CounterTracker::new(CounterWidth::Bits64, None);
        assert_eq!(t.update(reading(100, 0, None)), Rate::First);
    }

    #[test]
    fn simple_rate() {
        let r = two(
            CounterWidth::Bits64,
            None,
            reading(1_000, 0, None),
            reading(7_000, 60_000, None),
        );
        assert_eq!(r, Rate::PerSecond(100.0));
    }

    #[test]
    fn fractional_interval() {
        let r = two(
            CounterWidth::Bits64,
            None,
            reading(0, 0, None),
            reading(1_000, 500, None),
        );
        assert_eq!(r, Rate::PerSecond(2_000.0));
    }

    #[test]
    fn unchanged_counter_is_zero() {
        let r = two(
            CounterWidth::Bits32,
            None,
            reading(5, 0, None),
            reading(5, 10_000, None),
        );
        assert_eq!(r, Rate::PerSecond(0.0));
    }

    #[test]
    fn counter32_wrap_confirmed_by_uptime() {
        let r = two(
            CounterWidth::Bits32,
            None,
            reading(u64::from(u32::MAX) - 99, 0, Some(1_000)),
            reading(100, 10_000, Some(2_000)),
        );
        // 100 + 100 counts in 10 s
        assert_eq!(r, Rate::PerSecond(20.0));
    }

    #[test]
    fn counter32_wrap_confirmed_by_plausible_rate() {
        let r = two(
            CounterWidth::Bits32,
            Some(1_000.0),
            reading(u64::from(u32::MAX) - 99, 0, None),
            reading(100, 10_000, None),
        );
        assert_eq!(r, Rate::PerSecond(20.0));
    }

    #[test]
    fn counter32_decrease_without_evidence_is_skipped() {
        let r = two(
            CounterWidth::Bits32,
            None,
            reading(u64::from(u32::MAX) - 99, 0, None),
            reading(100, 10_000, None),
        );
        assert_eq!(r, Rate::Skipped(SkipReason::CounterReset));
    }

    #[test]
    fn counter32_reset_with_implausible_wrap_rate_is_skipped() {
        // A "wrap" worth ~295 MB in 10 s on a link limited to 1 MB/s: really a reset.
        let r = two(
            CounterWidth::Bits32,
            Some(1_000_000.0),
            reading(4_000_000_000, 0, None),
            reading(10, 10_000, None),
        );
        assert_eq!(r, Rate::Skipped(SkipReason::CounterReset));
    }

    #[test]
    fn counter64_decrease_is_reset() {
        let r = two(
            CounterWidth::Bits64,
            None,
            reading(1_000_000, 0, Some(10)),
            reading(10, 10_000, Some(20)),
        );
        assert_eq!(r, Rate::Skipped(SkipReason::CounterReset));
    }

    #[test]
    fn uptime_decrease_means_restart_even_if_counter_grew() {
        let r = two(
            CounterWidth::Bits64,
            None,
            reading(10, 0, Some(500_000)),
            reading(99_999, 10_000, Some(100)),
        );
        assert_eq!(r, Rate::Skipped(SkipReason::DeviceRestarted));
    }

    #[test]
    fn time_must_advance() {
        for second in [0, -5] {
            let r = two(
                CounterWidth::Bits64,
                None,
                reading(1, 1_000, None),
                reading(2, 1_000 + second, None),
            );
            assert_eq!(r, Rate::Skipped(SkipReason::NonMonotonicTime));
        }
    }

    #[test]
    fn value_wider_than_counter_is_invalid() {
        let r = two(
            CounterWidth::Bits32,
            None,
            reading(1, 0, None),
            reading(u64::from(u32::MAX) + 1, 1_000, None),
        );
        assert_eq!(r, Rate::Skipped(SkipReason::InvalidValue));
    }

    #[test]
    fn implausible_jump_is_skipped() {
        let r = two(
            CounterWidth::Bits64,
            Some(1_000.0),
            reading(0, 0, None),
            reading(1_000_000, 10_000, None),
        );
        assert_eq!(r, Rate::Skipped(SkipReason::ImplausibleRate));
    }

    #[test]
    fn skip_costs_one_sample_only() {
        let mut t = CounterTracker::new(CounterWidth::Bits64, None);
        assert_eq!(t.update(reading(1_000, 0, None)), Rate::First);
        assert!(matches!(
            t.update(reading(5, 10_000, None)),
            Rate::Skipped(_)
        ));
        // The post-reset reading is the new baseline.
        assert_eq!(t.update(reading(105, 20_000, None)), Rate::PerSecond(10.0));
    }

    #[test]
    fn uptime_missing_on_one_side_gives_no_confirmation() {
        let r = two(
            CounterWidth::Bits32,
            None,
            reading(u64::from(u32::MAX), 0, Some(1)),
            reading(0, 1_000, None),
        );
        assert_eq!(r, Rate::Skipped(SkipReason::CounterReset));
    }
}
