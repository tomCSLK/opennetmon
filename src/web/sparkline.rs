//! Small charts of the last hour for the sensor list, drawn on the server as SVG: no script, no inline
//! style (CSP), no extra request.

use std::fmt::Write;

use crate::status::Status;
use crate::web::overview::Segment;

/// Size of the drawing area; the SVG stretches it to its CSS size.
pub const WIDTH: f64 = 120.0;
pub const HEIGHT: f64 = 28.0;
/// Room for the stroke at the top and bottom edge.
const PAD: f64 = 2.0;
pub const WINDOW_MS: i64 = 3_600_000;
const MIN_BUCKET_MS: i64 = 60_000;
const LINE_CLASSES: [&str; 4] = ["spark-1", "spark-2", "spark-3", "spark-4"];

/// One line of a chart: SVG path data and a colour class.
pub struct Line {
    pub d: String,
    pub class: &'static str,
}

/// What the sensor list shows for the last hour.
pub enum Trend {
    /// Nothing measured yet.
    Empty,
    /// Measured values, one line per channel.
    Lines(Vec<Line>),
    /// Status over time (status values, uptime): coloured like the status bars of the overview.
    Band(Vec<Segment>),
}

/// Bucket length and count for the window: at least two polls per bucket, so steady polling
/// leaves no gaps (2 minutes for the usual 60-second interval, longer for slow ones).
pub fn buckets(interval_secs: u32) -> (i64, usize) {
    let bucket = MIN_BUCKET_MS.max(i64::from(interval_secs) * 2_000);
    let count = (WINDOW_MS + bucket - 1) / bucket;
    (bucket, usize::try_from(count).unwrap_or(0))
}

/// Lines for series of bucket averages on one shared scale. The scale starts at 0 (or at the
/// minimum, if a value is negative), so a steady value stays a steady line and small jitter does
/// not look like a spike. Empty buckets break the line.
pub fn lines(series: &[Vec<Option<f64>>]) -> Vec<Line> {
    let finite = |v: &&Option<f64>| v.is_some_and(f64::is_finite);
    let (lo, hi) = series
        .iter()
        .flat_map(|s| s.iter().filter(finite).flatten())
        .fold((0.0_f64, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        });
    if hi == f64::NEG_INFINITY {
        return Vec::new();
    }
    let span = if hi > lo { hi - lo } else { 1.0 };
    series
        .iter()
        .zip(LINE_CLASSES.iter().cycle())
        .filter_map(|(values, class)| {
            let n = values.len() as f64;
            let mut d = String::new();
            let mut run = 0;
            for (i, v) in values.iter().enumerate() {
                match v.filter(|v| v.is_finite()) {
                    Some(v) => {
                        let x = (i as f64 + 0.5) * WIDTH / n;
                        let y = HEIGHT - PAD - (v - lo) / span * (HEIGHT - 2.0 * PAD);
                        let command = match (run, d.is_empty()) {
                            (0, true) => "M",
                            (0, false) => " M",
                            _ => " L",
                        };
                        let _ = write!(d, "{command}{x:.1} {y:.1}");
                        run += 1;
                    }
                    None => {
                        close_lone_point(&mut d, run);
                        run = 0;
                    }
                }
            }
            close_lone_point(&mut d, run);
            (!d.is_empty()).then_some(Line { d, class })
        })
        .collect()
}

/// A single point between gaps would draw nothing: a zero-length segment shows it as a dot
/// (round line caps).
fn close_lone_point(d: &mut String, run: usize) {
    if run == 1 {
        d.push_str(" l0 0");
    }
}

/// Status segments from `from_ms` to `to_ms`: `start` is the status at `from_ms`, `changes` the
/// later changes, oldest first. Time without any status stays empty (the bar background).
pub fn band(
    start: Option<Status>,
    changes: &[(i64, Status)],
    from_ms: i64,
    to_ms: i64,
) -> Vec<Segment> {
    let span = (to_ms - from_ms) as f64;
    let mut out = Vec::new();
    if span <= 0.0 {
        return out;
    }
    let mut push = |status: Option<Status>, a: i64, b: i64| {
        if let Some(status) = status
            && b > a
        {
            out.push(Segment {
                x: format!("{:.3}%", (a - from_ms) as f64 * 100.0 / span),
                width: format!("{:.3}%", (b - a) as f64 * 100.0 / span),
                class: segment_class(status),
            });
        }
    };
    let (mut current, mut since) = (start, from_ms);
    for &(ts, status) in changes {
        let ts = ts.clamp(from_ms, to_ms);
        push(current, since, ts);
        (current, since) = (Some(status), ts);
    }
    push(current, since, to_ms);
    out
}

fn segment_class(status: Status) -> &'static str {
    match status {
        Status::Up => "seg-up",
        Status::Warning => "seg-warning",
        Status::Down => "seg-down",
        Status::Unknown => "seg-unknown",
        Status::Paused => "seg-paused",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_cover_the_hour_with_at_least_two_polls_each() {
        assert_eq!(buckets(60), (2 * 60_000, 30));
        assert_eq!(buckets(10), (60_000, 60));
        assert_eq!(buckets(300), (10 * 60_000, 6));
        // Hourly polling: a single bucket (a dot) covering the hour.
        assert_eq!(buckets(3_600), (2 * 3_600_000, 1));
        // A bucket length that does not divide the hour still covers all of it.
        let (bucket, count) = buckets(700);
        assert!(bucket * i64::try_from(count).unwrap() >= WINDOW_MS);
    }

    #[test]
    fn a_line_runs_from_left_to_right_on_a_scale_from_zero() {
        let lines = lines(&[vec![Some(0.0), Some(5.0), Some(10.0), Some(10.0)]]);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].class, "spark-1");
        assert_eq!(lines[0].d, "M15.0 26.0 L45.0 14.0 L75.0 2.0 L105.0 2.0");
    }

    #[test]
    fn channels_share_one_scale_and_get_their_own_colour() {
        let lines = lines(&[vec![Some(10.0), Some(10.0)], vec![Some(5.0), None]]);
        assert_eq!(lines[0].d, "M30.0 2.0 L90.0 2.0");
        // Half of the maximum, a single point: drawn as a dot.
        assert_eq!(lines[1].d, "M30.0 14.0 l0 0");
        assert_eq!(lines[1].class, "spark-2");
    }

    #[test]
    fn gaps_break_the_line_and_empty_series_draw_nothing() {
        let lines = lines(&[
            vec![Some(1.0), Some(1.0), None, Some(1.0), Some(1.0)],
            vec![None, None, None, None, None],
        ]);
        assert_eq!(lines.len(), 1, "the empty series has no line");
        assert_eq!(lines[0].d, "M12.0 2.0 L36.0 2.0 M84.0 2.0 L108.0 2.0");
        assert!(super::lines(&[vec![None, None]]).is_empty());
        assert!(super::lines(&[]).is_empty());
    }

    #[test]
    fn negative_values_move_the_baseline_and_constant_zero_lies_flat() {
        let lines = lines(&[vec![Some(-10.0), Some(10.0)]]);
        assert_eq!(lines[0].d, "M30.0 26.0 L90.0 2.0");
        let zero = super::lines(&[vec![Some(0.0), Some(0.0)]]);
        assert_eq!(zero[0].d, "M30.0 26.0 L90.0 26.0");
        let odd = super::lines(&[vec![Some(f64::NAN), Some(1.0)]]);
        assert_eq!(odd[0].d, "M90.0 2.0 l0 0");
    }

    #[test]
    fn the_band_follows_the_status_changes() {
        let segments = band(
            Some(Status::Up),
            &[(250, Status::Down), (750, Status::Up)],
            0,
            1_000,
        );
        let shown: Vec<_> = segments
            .iter()
            .map(|s| (s.x.as_str(), s.width.as_str(), s.class))
            .collect();
        assert_eq!(
            shown,
            [
                ("0.000%", "25.000%", "seg-up"),
                ("25.000%", "50.000%", "seg-down"),
                ("75.000%", "25.000%", "seg-up")
            ]
        );
    }

    #[test]
    fn time_before_the_first_status_stays_empty() {
        let segments = band(None, &[(500, Status::Warning)], 0, 1_000);
        assert_eq!(segments.len(), 1);
        assert_eq!(
            (segments[0].x.as_str(), segments[0].class),
            ("50.000%", "seg-warning")
        );
        assert!(band(None, &[], 0, 1_000).is_empty());
        assert!(band(Some(Status::Up), &[], 1_000, 1_000).is_empty());
    }
}
