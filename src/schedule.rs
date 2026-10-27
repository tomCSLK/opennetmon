//! Maintenance windows like the schedules of PRTG: a named schedule has windows, weekly ("Monday to
//! Friday 03:00, 15 minutes") or once ("Saturday 10:00 to 11:00"). The objects a schedule applies to
//! (inherited from group to subgroup, device and sensor like a dependency) are paused during its
//! windows, see [`crate::pause::Pauses`]. Weekly windows are in the time zone of the server (config
//! `time_zone`, else the system's), so they keep their local time across daylight saving changes.
//! Storage: `storage::schedules`.

use std::sync::OnceLock;

use jiff::Timestamp;
use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;

use crate::i18n::Msg;

pub const MAX_NAME_LEN: usize = 100;
/// Longest weekly window: a whole week.
pub const MAX_DURATION_MIN: u32 = 7 * 24 * 60;
/// All weekdays in [`Window::Weekly::weekdays`].
pub const EVERY_DAY: u8 = 0b111_1111;

static TIME_ZONE: OnceLock<TimeZone> = OnceLock::new();

/// Sets the time zone of the schedules (startup code, before the first window is computed); `None` =
/// the system's. Errors if the name is unknown.
pub fn set_time_zone(name: Option<&str>) -> Result<(), String> {
    let tz = match name {
        Some(name) => TimeZone::get(name).map_err(|e| format!("time_zone {name:?}: {e}"))?,
        None => system_zone(),
    };
    // A second call (tests) keeps the first zone.
    let _ = TIME_ZONE.set(tz);
    Ok(())
}

fn system_zone() -> TimeZone {
    TimeZone::try_system().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "time zone of the system unknown, schedules use UTC");
        TimeZone::UTC
    })
}

/// The time zone of the schedules.
pub fn time_zone() -> &'static TimeZone {
    TIME_ZONE.get_or_init(system_zone)
}

/// The name of a zone for the pages ("Europe/Berlin"; a zone without a name shows as its offset now).
pub fn zone_name(tz: &TimeZone) -> String {
    tz.iana_name().map_or_else(
        || Timestamp::now().to_zoned(tz.clone()).offset().to_string(),
        str::to_owned,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    /// On the days of `weekdays` (bit 0 = Monday … bit 6 = Sunday), starting `start_minute` after
    /// local midnight, lasting `duration_minutes`.
    Weekly {
        weekdays: u8,
        start_minute: u16,
        duration_minutes: u32,
    },
    Once {
        start_ms: i64,
        end_ms: i64,
    },
}

fn bit(date: Date) -> u8 {
    1 << date.weekday().to_monday_zero_offset()
}

/// The start of a weekly window on `date` (a time skipped by daylight saving starts at the first
/// time after the gap; a repeated time at its first occurrence).
fn start_on(date: Date, start_minute: u16, tz: &TimeZone) -> Option<i64> {
    let hour = i8::try_from(start_minute / 60).ok()?;
    let minute = i8::try_from(start_minute % 60).ok()?;
    let zoned = date.at(hour, minute, 0, 0).to_zoned(tz.clone()).ok()?;
    Some(zoned.timestamp().as_millisecond())
}

fn local_date(ms: i64, tz: &TimeZone) -> Option<Date> {
    Some(
        Timestamp::from_millisecond(ms)
            .ok()?
            .to_zoned(tz.clone())
            .date(),
    )
}

impl Window {
    /// The end of the window if it is open at `now_ms`.
    pub fn active_until(&self, now_ms: i64, tz: &TimeZone) -> Option<i64> {
        match *self {
            Self::Once { start_ms, end_ms } => {
                (start_ms <= now_ms && now_ms < end_ms).then_some(end_ms)
            }
            Self::Weekly {
                weekdays,
                start_minute,
                duration_minutes,
            } => {
                let duration_ms = i64::from(duration_minutes) * 60_000;
                // Windows that started on one of the last days and are still open; a window of a
                // whole week can still be open eight days after its start day began.
                let mut date = local_date(now_ms, tz)?;
                let mut best: Option<i64> = None;
                for _ in 0..=(duration_minutes / 1440 + 1) {
                    if weekdays & bit(date) != 0
                        && let Some(start) = start_on(date, start_minute, tz)
                    {
                        let end = start.saturating_add(duration_ms);
                        if start <= now_ms && now_ms < end {
                            best = best.max(Some(end));
                        }
                    }
                    date = date.yesterday().ok()?;
                }
                best
            }
        }
    }

    /// The next start after `now_ms`, if there is one.
    pub fn next_start(&self, now_ms: i64, tz: &TimeZone) -> Option<i64> {
        match *self {
            Self::Once { start_ms, .. } => (start_ms > now_ms).then_some(start_ms),
            Self::Weekly {
                weekdays,
                start_minute,
                ..
            } => {
                let mut date = local_date(now_ms, tz)?;
                for _ in 0..=7 {
                    if weekdays & bit(date) != 0
                        && let Some(start) = start_on(date, start_minute, tz)
                        && start > now_ms
                    {
                        return Some(start);
                    }
                    date = date.tomorrow().ok()?;
                }
                None
            }
        }
    }

    /// Whether the window can never open again (a single window that has ended).
    pub fn is_over(&self, now_ms: i64) -> bool {
        matches!(*self, Self::Once { end_ms, .. } if end_ms <= now_ms)
    }
}

/// The end of the open windows at `now_ms` (the latest, if several overlap).
pub fn active_until(windows: &[Window], now_ms: i64, tz: &TimeZone) -> Option<i64> {
    windows
        .iter()
        .filter_map(|w| w.active_until(now_ms, tz))
        .max()
}

/// The next start of any window after `now_ms`.
pub fn next_start(windows: &[Window], now_ms: i64, tz: &TimeZone) -> Option<i64> {
    windows
        .iter()
        .filter_map(|w| w.next_start(now_ms, tz))
        .min()
}

/// The schedule setting of a group, device or sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Assignment {
    /// Like the object above (for a group without parent: none).
    #[default]
    Parent,
    Schedule(i64),
    /// No schedule, whatever the objects above say.
    None,
}

impl Assignment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Parent => "parent",
            Self::Schedule(_) => "schedule",
            Self::None => "none",
        }
    }

    pub fn schedule_id(self) -> Option<i64> {
        match self {
            Self::Schedule(id) => Some(id),
            _ => None,
        }
    }
}

pub fn validate_name(name: &str) -> Result<String, Msg> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Msg::new("schedule-name-required"));
    }
    if name.chars().count() > MAX_NAME_LEN || name.chars().any(char::is_control) {
        return Err(Msg::new("schedule-name-invalid").with("max", MAX_NAME_LEN));
    }
    Ok(name.to_owned())
}

/// `03:00` (the value of `<input type="time">`, seconds allowed) as minutes after midnight.
pub fn parse_time(text: &str) -> Result<u16, Msg> {
    let invalid = || Msg::new("schedule-time-invalid");
    let mut parts = text.trim().split(':');
    let hour: u16 = parts
        .next()
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    let minute: u16 = parts
        .next()
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    if parts.next().is_some_and(|s| s.parse::<u16>().is_err()) || parts.next().is_some() {
        return Err(invalid());
    }
    if hour > 23 || minute > 59 {
        return Err(invalid());
    }
    Ok(hour * 60 + minute)
}

/// `03:00` for minutes after midnight.
pub fn format_time(start_minute: u16) -> String {
    format!("{:02}:{:02}", start_minute / 60, start_minute % 60)
}

/// The duration field in minutes, 1 to [`MAX_DURATION_MIN`].
pub fn parse_duration(text: &str) -> Result<u32, Msg> {
    match text.trim().parse::<u32>() {
        Ok(m) if (1..=MAX_DURATION_MIN).contains(&m) => Ok(m),
        _ => Err(Msg::new("schedule-duration-invalid").with("max", MAX_DURATION_MIN)),
    }
}

/// The weekday checkboxes (`1` = Monday … `7` = Sunday) as a bit mask; at least one.
pub fn parse_weekdays(values: &[String]) -> Result<u8, Msg> {
    let mut mask = 0u8;
    for v in values {
        match v.trim().parse::<u8>() {
            Ok(day @ 1..=7) => mask |= 1 << (day - 1),
            _ => return Err(Msg::new("schedule-weekdays-required")),
        }
    }
    if mask == 0 {
        return Err(Msg::new("schedule-weekdays-required"));
    }
    Ok(mask)
}

/// The value of `<input type="datetime-local">` (`2026-10-31T10:00`) in zone `tz`, as UTC milliseconds.
pub fn parse_local(text: &str, tz: &TimeZone) -> Result<i64, Msg> {
    let invalid = || Msg::new("schedule-datetime-invalid");
    let dt: DateTime = text.trim().parse().map_err(|_| invalid())?;
    let zoned = dt.to_zoned(tz.clone()).map_err(|_| invalid())?;
    Ok(zoned.timestamp().as_millisecond())
}

/// UTC milliseconds as the value of `<input type="datetime-local">` in zone `tz`.
pub fn format_local(ms: i64, tz: &TimeZone) -> String {
    Timestamp::from_millisecond(ms)
        .map(|ts| {
            ts.to_zoned(tz.clone())
                .strftime("%Y-%m-%dT%H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// A single window from the two fields; the end must be after the start.
pub fn parse_once(start: &str, end: &str, tz: &TimeZone) -> Result<Window, Msg> {
    let start_ms = parse_local(start, tz)?;
    let end_ms = parse_local(end, tz)?;
    if end_ms <= start_ms {
        return Err(Msg::new("schedule-end-before-start"));
    }
    Ok(Window::Once { start_ms, end_ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn berlin() -> TimeZone {
        TimeZone::get("Europe/Berlin").unwrap()
    }

    fn at(text: &str) -> i64 {
        parse_local(text, &berlin()).unwrap()
    }

    const WEEKDAYS: u8 = 0b001_1111; // Monday to Friday

    #[test]
    fn a_weekly_window_is_open_on_its_days_only() {
        let tz = berlin();
        let w = Window::Weekly {
            weekdays: WEEKDAYS,
            start_minute: 3 * 60,
            duration_minutes: 15,
        };
        // 2026-10-27 is a Tuesday.
        assert_eq!(
            w.active_until(at("2026-10-27T03:00"), &tz),
            Some(at("2026-10-27T03:15"))
        );
        assert_eq!(
            w.active_until(at("2026-10-27T03:14"), &tz),
            Some(at("2026-10-27T03:15"))
        );
        assert_eq!(w.active_until(at("2026-10-27T03:15"), &tz), None);
        assert_eq!(w.active_until(at("2026-10-27T02:59"), &tz), None);
        // Saturday: not one of its days.
        assert_eq!(w.active_until(at("2026-10-31T03:05"), &tz), None);
        assert_eq!(
            w.next_start(at("2026-10-27T03:05"), &tz),
            Some(at("2026-10-28T03:00"))
        );
        assert_eq!(
            w.next_start(at("2026-10-30T04:00"), &tz),
            Some(at("2026-11-02T03:00"))
        );
    }

    #[test]
    fn a_window_over_midnight_belongs_to_the_day_it_starts() {
        let tz = berlin();
        let w = Window::Weekly {
            weekdays: 1 << 4, // Friday
            start_minute: 23 * 60,
            duration_minutes: 120,
        };
        // Friday 2026-10-30 23:00 to Saturday 01:00.
        assert_eq!(
            w.active_until(at("2026-10-31T00:30"), &tz),
            Some(at("2026-10-31T01:00"))
        );
        assert_eq!(w.active_until(at("2026-10-30T00:30"), &tz), None);
        let week = Window::Weekly {
            weekdays: 1, // Monday, a whole week
            start_minute: 0,
            duration_minutes: MAX_DURATION_MIN,
        };
        assert_eq!(
            week.active_until(at("2026-11-01T23:59"), &tz),
            Some(at("2026-11-02T00:00"))
        );
    }

    #[test]
    fn local_times_stay_across_daylight_saving_changes() {
        let tz = berlin();
        let daily = Window::Weekly {
            weekdays: EVERY_DAY,
            start_minute: 3 * 60,
            duration_minutes: 30,
        };
        // Clocks went back on Sunday 2026-10-25 (03:00 summer time became 02:00): 03:00 is still 03:00.
        let sunday = at("2026-10-25T03:00");
        assert_eq!(daily.active_until(sunday, &tz), Some(sunday + 30 * 60_000));
        assert_eq!(daily.next_start(at("2026-10-24T12:00"), &tz), Some(sunday));
        // The previous day's window was in summer time: one hour more than a day apart in UTC.
        assert_eq!(sunday - at("2026-10-24T03:00"), 25 * 3_600_000);
        // On 2026-03-29 02:30 does not exist (02:00 became 03:00): the window starts after the gap.
        let skipped = Window::Weekly {
            weekdays: EVERY_DAY,
            start_minute: 2 * 60 + 30,
            duration_minutes: 10,
        };
        let start = skipped.next_start(at("2026-03-29T00:00"), &tz).unwrap();
        assert_eq!(format_local(start, &tz), "2026-03-29T03:30");
    }

    #[test]
    fn a_single_window_opens_once() {
        let tz = berlin();
        let w = parse_once("2026-10-31T10:00", "2026-10-31T11:00", &tz).unwrap();
        assert_eq!(w.active_until(at("2026-10-31T09:59"), &tz), None);
        assert_eq!(
            w.active_until(at("2026-10-31T10:00"), &tz),
            Some(at("2026-10-31T11:00"))
        );
        assert_eq!(
            w.next_start(at("2026-10-30T00:00"), &tz),
            Some(at("2026-10-31T10:00"))
        );
        assert_eq!(w.next_start(at("2026-10-31T10:30"), &tz), None);
        assert!(w.is_over(at("2026-10-31T11:00")));
        assert_eq!(
            parse_once("2026-10-31T11:00", "2026-10-31T10:00", &tz),
            Err(Msg::new("schedule-end-before-start"))
        );
    }

    #[test]
    fn overlapping_windows_end_with_the_last() {
        let tz = berlin();
        let windows = [
            Window::Weekly {
                weekdays: EVERY_DAY,
                start_minute: 3 * 60,
                duration_minutes: 15,
            },
            parse_once("2026-10-27T02:00", "2026-10-27T04:00", &tz).unwrap(),
        ];
        assert_eq!(
            active_until(&windows, at("2026-10-27T03:05"), &tz),
            Some(at("2026-10-27T04:00"))
        );
        assert_eq!(
            next_start(&windows, at("2026-10-27T01:00"), &tz),
            Some(at("2026-10-27T02:00"))
        );
    }

    #[test]
    fn form_fields_are_checked() {
        assert_eq!(parse_time("03:00"), Ok(180));
        assert_eq!(parse_time("23:59:00"), Ok(1439));
        for bad in ["", "24:00", "3", "03:60", "a:b", "03:00:00:00"] {
            assert!(parse_time(bad).is_err(), "{bad}");
        }
        assert_eq!(format_time(185), "03:05");
        assert_eq!(parse_duration("15"), Ok(15));
        for bad in ["0", "", "-1", "10081"] {
            assert!(parse_duration(bad).is_err(), "{bad}");
        }
        let days =
            |d: &[&str]| parse_weekdays(&d.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        assert_eq!(days(&["1", "7"]), Ok(0b100_0001));
        assert!(days(&[]).is_err());
        assert!(days(&["8"]).is_err());
        assert!(validate_name("  ").is_err());
        assert_eq!(validate_name(" Nachts "), Ok("Nachts".to_owned()));
        assert!(parse_local("morgen", &berlin()).is_err());
    }
}
