//! Availability like the reports of PRTG: how long each sensor spent in each status, counted per hour
//! when it is polled (`storage::status::record_poll`, tables `availability_1h`/`availability_1d`).
//! Available = OK or warning, unavailable = down; paused time, unknown time and gaps without polls
//! do not count. A device is as available as its master sensor, else its ping sensor, else the mean
//! of its sensors.

use crate::status::Status;

pub const HOUR_MS: i64 = 3_600_000;
pub const DAY_MS: i64 = 24 * HOUR_MS;
/// A pause between two polls longer than this many intervals is a gap (service stopped, host off):
/// it counts as no data instead of the status before.
pub const GAP_INTERVALS: i64 = 3;

/// Milliseconds per status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Totals {
    pub up_ms: i64,
    pub warning_ms: i64,
    pub down_ms: i64,
    pub unknown_ms: i64,
    pub paused_ms: i64,
}

impl Totals {
    pub fn add(&mut self, status: Status, ms: i64) {
        let slot = match status {
            Status::Up => &mut self.up_ms,
            Status::Warning => &mut self.warning_ms,
            Status::Down => &mut self.down_ms,
            Status::Unknown => &mut self.unknown_ms,
            Status::Paused => &mut self.paused_ms,
        };
        *slot = slot.saturating_add(ms);
    }

    pub fn merge(&mut self, other: &Self) {
        self.up_ms = self.up_ms.saturating_add(other.up_ms);
        self.warning_ms = self.warning_ms.saturating_add(other.warning_ms);
        self.down_ms = self.down_ms.saturating_add(other.down_ms);
        self.unknown_ms = self.unknown_ms.saturating_add(other.unknown_ms);
        self.paused_ms = self.paused_ms.saturating_add(other.paused_ms);
    }

    /// The time that counts: OK, warning and down.
    pub fn counted_ms(&self) -> i64 {
        self.up_ms
            .saturating_add(self.warning_ms)
            .saturating_add(self.down_ms)
    }

    /// Availability in percent; `None` without counted time.
    pub fn percent(&self) -> Option<f64> {
        let counted = self.counted_ms();
        (counted > 0).then(|| (self.up_ms + self.warning_ms) as f64 * 100.0 / counted as f64)
    }
}

/// The parts of `from_ms..to_ms` per hour: `(start of the hour, milliseconds in it)`.
pub fn split_hours(from_ms: i64, to_ms: i64) -> Vec<(i64, i64)> {
    let mut parts = Vec::new();
    let mut at = from_ms;
    while at < to_ms {
        let hour = at.div_euclid(HOUR_MS) * HOUR_MS;
        let end = (hour + HOUR_MS).min(to_ms);
        parts.push((hour, end - at));
        at = end;
    }
    parts
}

/// The time between two polls that is added to the status before, or `None` for a gap (or a clock
/// that went backwards).
pub fn counted_span(last_poll_ms: i64, now_ms: i64, interval_secs: u32) -> Option<(i64, i64)> {
    let limit = i64::from(interval_secs) * 1_000 * GAP_INTERVALS;
    let span = now_ms - last_poll_ms;
    (span > 0 && span <= limit).then_some((last_poll_ms, now_ms))
}

/// What the availability of a device is based on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    Master(i64),
    Ping(i64),
    /// The mean of the sensors that have counted time.
    Mean,
}

/// A sensor of a device as the device's availability sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Part {
    pub sensor_id: i64,
    pub master: bool,
    pub ping: bool,
    pub totals: Totals,
}

/// The availability of a device: the percentage, the downtime (not for a mean) and its basis.
pub fn device(parts: &[Part]) -> (Option<f64>, Option<i64>, Basis) {
    let pick = parts
        .iter()
        .find(|p| p.master)
        .map(|p| (p, Basis::Master(p.sensor_id)))
        .or_else(|| {
            parts
                .iter()
                .find(|p| p.ping)
                .map(|p| (p, Basis::Ping(p.sensor_id)))
        });
    if let Some((p, basis)) = pick {
        let down = (p.totals.counted_ms() > 0).then_some(p.totals.down_ms);
        return (p.totals.percent(), down, basis);
    }
    let values: Vec<f64> = parts.iter().filter_map(|p| p.totals.percent()).collect();
    let mean = (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64);
    (mean, None, Basis::Mean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_are_split_at_hour_boundaries() {
        assert_eq!(split_hours(0, 0), []);
        assert_eq!(split_hours(1_000, 61_000), [(0, 60_000)]);
        let from = HOUR_MS - 30_000;
        assert_eq!(
            split_hours(from, 2 * HOUR_MS + 10_000),
            [(0, 30_000), (HOUR_MS, HOUR_MS), (2 * HOUR_MS, 10_000)]
        );
        assert_eq!(split_hours(HOUR_MS, 2 * HOUR_MS), [(HOUR_MS, HOUR_MS)]);
    }

    #[test]
    fn gaps_do_not_count() {
        assert_eq!(counted_span(0, 60_000, 60), Some((0, 60_000)));
        assert_eq!(counted_span(0, 180_000, 60), Some((0, 180_000)));
        assert_eq!(
            counted_span(0, 180_001, 60),
            None,
            "more than three intervals"
        );
        assert_eq!(counted_span(5_000, 4_000, 60), None, "clock went back");
        assert_eq!(counted_span(5_000, 5_000, 60), None);
    }

    #[test]
    fn availability_counts_ok_and_warning_against_down_only() {
        let mut t = Totals::default();
        assert_eq!(t.percent(), None);
        t.add(Status::Up, 7_000);
        t.add(Status::Warning, 2_000);
        t.add(Status::Down, 1_000);
        t.add(Status::Paused, 50_000);
        t.add(Status::Unknown, 9_000);
        assert_eq!(t.counted_ms(), 10_000);
        assert_eq!(t.percent(), Some(90.0));
        let mut sum = t;
        sum.merge(&t);
        assert_eq!((sum.down_ms, sum.percent()), (2_000, Some(90.0)));
    }

    #[test]
    fn a_device_follows_its_master_then_its_ping_then_the_mean() {
        let totals = |up, down| Totals {
            up_ms: up,
            down_ms: down,
            ..Totals::default()
        };
        let part = |id, master, ping, t| Part {
            sensor_id: id,
            master,
            ping,
            totals: t,
        };
        let ping = part(1, false, true, totals(90, 10));
        let port = part(2, false, false, totals(50, 50));
        assert_eq!(
            device(&[port, ping]),
            (Some(90.0), Some(10), Basis::Ping(1))
        );
        let master = part(3, true, false, totals(100, 0));
        assert_eq!(
            device(&[ping, master, port]),
            (Some(100.0), Some(0), Basis::Master(3))
        );
        let other = part(4, false, false, totals(100, 0));
        assert_eq!(device(&[port, other]), (Some(75.0), None, Basis::Mean));
        let empty = part(5, false, false, Totals::default());
        assert_eq!(device(&[empty]), (None, None, Basis::Mean));
        let unpolled_ping = part(6, false, true, Totals::default());
        assert_eq!(device(&[unpolled_ping]), (None, None, Basis::Ping(6)));
    }
}
