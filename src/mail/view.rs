//! Building blocks of the HTML mails (alerts, test mail): status tones and chips, facts, buttons,
//! history bars, local times and the frame around every mail. The templates are in
//! `templates/mail/`; they use tables and styles on the elements, because mail programs ignore most
//! style sheets (the "no inline styles" rule of the web pages, which comes from their CSP, does not
//! apply to mails). Everything a mail shows is in it: the logo is an inline image (`cid:`), there are
//! no external resources and no scripts.

use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::i18n::Lang;
use crate::mail::InlineImage;
use crate::status::{Status, Thresholds};

/// Content id of the logo in the HTML part.
pub const LOGO_CID: &str = "logo@opennetmon";
/// "Logo C" as an outline (monitor with pulse line, like the macro `logo` in `templates/icons.html`),
/// 66×66 with a transparent background, shown at 22×22. Not under `static/`, which is served.
const LOGO_PNG: &[u8] = include_bytes!("logo.png");

/// The logo as an inline image of the mail.
pub fn logo() -> InlineImage {
    InlineImage {
        content_id: LOGO_CID.to_owned(),
        content_type: "image/png",
        data: LOGO_PNG.to_vec(),
    }
}

/// Content id of the history picture of a single change.
pub const CHART_CID: &str = "chart@opennetmon";

/// The history picture ([`crate::chart_png`]) as an inline image of the mail.
pub fn chart_image(png: Vec<u8>) -> InlineImage {
    InlineImage {
        content_id: CHART_CID.to_owned(),
        content_type: "image/png",
        data: png,
    }
}

/// Colour of the history bars (the first series colour of the charts).
const SERIES: &str = "#1c5ea8";
/// Bars beyond a warning limit (darker than the warning text colour, readable on white).
const WARN_BAR: &str = "#b07d00";

/// Status colours of a mail element: foreground, background, border and the class the dark mode
/// rules of `templates/mail/base.html` use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Down,
    Warning,
    Up,
    /// Paused or unknown.
    Off,
}

impl Tone {
    pub fn of(status: Status) -> Self {
        match status {
            Status::Down => Self::Down,
            Status::Warning => Self::Warning,
            Status::Up => Self::Up,
            Status::Unknown | Status::Paused => Self::Off,
        }
    }

    /// Symbol shown with the word, so the status does not depend on colour alone.
    pub fn sym(self) -> &'static str {
        match self {
            Self::Down => "✕",
            Self::Warning => "▲",
            Self::Up => "✓",
            Self::Off => "‖",
        }
    }

    pub fn fg(self) -> &'static str {
        match self {
            Self::Down => "#a1132c",
            Self::Warning => "#7a5600",
            Self::Up => "#1d6b3c",
            Self::Off => "#46596d",
        }
    }

    pub fn bg(self) -> &'static str {
        match self {
            Self::Down => "#fbe8eb",
            Self::Warning => "#fbf3d9",
            Self::Up => "#e6f3ea",
            Self::Off => "#edf1f5",
        }
    }

    pub fn line(self) -> &'static str {
        match self {
            Self::Down => "#e7a3ae",
            Self::Warning => "#dcc275",
            Self::Up => "#a8d2b8",
            Self::Off => "#b5c2cf",
        }
    }

    /// Strong colour for bands and status bars.
    pub fn band(self) -> &'static str {
        match self {
            Self::Warning => WARN_BAR,
            other => other.fg(),
        }
    }

    pub fn cls(self) -> &'static str {
        match self {
            Self::Down => "err",
            Self::Warning => "warn",
            Self::Up => "ok",
            Self::Off => "off",
        }
    }
}

/// A status label: symbol and word in the colours of the status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub tone: Tone,
    pub text: String,
}

/// The head of a mail: status chip and time, title, and below it the group path and address (or a
/// note).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub chip: Chip,
    pub when: String,
    pub title: String,
    pub path: Vec<String>,
    pub host: String,
    /// A line of text in pieces, `true` = bold.
    pub note: Vec<(String, bool)>,
}

/// Text for HTML: a space after a digit becomes a no-break space, so "90 °C", "3 Std." or "2 Geräte"
/// never wrap between number and unit.
pub fn nb(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut after_digit = false;
    for c in text.chars() {
        if c == ' ' && after_digit {
            out.push('\u{a0}');
        } else {
            out.push(c);
        }
        after_digit = c.is_ascii_digit();
    }
    out
}

/// The template filter `nb` ([`nb`]); the result is escaped like any other text.
pub mod filters {
    #[askama::filter_fn]
    pub fn nb<T: std::fmt::Display>(text: T, _: &dyn askama::Values) -> askama::Result<String> {
        Ok(super::nb(&text.to_string()))
    }
}

/// A duration for people: seconds below a minute, whole minutes below a day, from a day on days and
/// hours.
pub fn duration(t: Lang, ms: i64) -> String {
    let secs = ms.max(0) / 1_000;
    let minutes = (secs + 30) / 60;
    match secs {
        0..60 => format!("{secs} s"),
        _ if minutes < 60 => t.get1("duration-minutes", "minutes", minutes),
        _ if minutes < 24 * 60 => t.get2(
            "duration-hours-minutes",
            "hours",
            minutes / 60,
            "minutes",
            minutes % 60,
        ),
        _ => {
            let hours = (secs + 1_800) / 3_600;
            match hours % 24 {
                0 => t.get1("mail-duration-days", "days", hours / 24),
                h => t.get2("mail-duration-days-hours", "days", hours / 24, "hours", h),
            }
        }
    }
}

/// A key figure: label, value and an optional line below.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub label: String,
    pub value: String,
    pub sub: String,
}

impl Fact {
    pub fn new(label: String, value: String, sub: String) -> Self {
        Self { label, value, sub }
    }
}

/// A link to the web interface, shown as a button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    pub label: String,
    pub href: String,
    pub primary: bool,
}

/// Links into the web interface, built from its address (`MailConfig.ui_url`, validated by
/// [`crate::mail::parse_ui_url`]) plus fixed paths and numeric ids only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Links {
    base: String,
}

impl Links {
    pub fn new(ui_url: Option<&str>) -> Option<Self> {
        ui_url.map(|base| Self {
            base: base.trim_end_matches('/').to_owned(),
        })
    }

    /// The host (and port) of the address, for the head of the mail.
    pub fn host(&self) -> String {
        let rest = self.base.split_once("://").map_or(&*self.base, |(_, r)| r);
        rest.split('/').next().unwrap_or_default().to_owned()
    }

    pub fn overview(&self) -> String {
        format!("{}/", self.base)
    }

    pub fn sensor(&self, id: i64) -> String {
        format!("{}/sensors/{id}", self.base)
    }

    pub fn sensor_settings(&self, id: i64) -> String {
        format!("{}/sensors/{id}/edit", self.base)
    }

    pub fn device(&self, id: i64) -> String {
        format!("{}/devices/{id}/sensors", self.base)
    }

    pub fn sensor_log(&self, id: i64) -> String {
        format!("{}/log?sensor={id}", self.base)
    }

    pub fn log(&self) -> String {
        format!("{}/log", self.base)
    }

    pub fn report_week(&self) -> String {
        format!("{}/reports?period=7d&view=sensors", self.base)
    }

    pub fn mail_settings(&self) -> String {
        format!("{}/settings/mail", self.base)
    }
}

/// One column of a history chart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bar {
    /// Height of the bar in pixels; 0 = no bar.
    pub height: u32,
    pub color: &'static str,
    /// Class for the dark mode rules.
    pub class: &'static str,
    /// Background of the whole column: a stretch of outage (or a failed poll).
    pub tint: Option<Tone>,
}

impl Bar {
    const EMPTY: Self = Self {
        height: 0,
        color: "",
        class: "",
        tint: None,
    };
}

/// An entry of the legend below a chart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegendItem {
    pub text: String,
    pub tone: Option<Tone>,
}

/// A history chart of HTML table cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chart {
    pub bars: Vec<Bar>,
    /// Height of the drawing area in pixels.
    pub height: u32,
    /// Times at the left and right edge.
    pub start: String,
    pub end: String,
    pub legend: Vec<LegendItem>,
}

impl Chart {
    pub fn has_bars(&self) -> bool {
        self.bars.iter().any(|b| b.height > 0 || b.tint.is_some())
    }
}

/// How bad a status is, for the worst status of a stretch.
fn rank(status: Status) -> u8 {
    match status {
        Status::Down => 4,
        Status::Warning => 3,
        Status::Unknown => 2,
        Status::Up => 1,
        Status::Paused => 0,
    }
}

/// The worst status of each of `count` buckets of `bucket_ms` from `from_ms`, from the status
/// periods (see [`crate::web::sparkline::periods`]); `None` where there was none.
pub fn bucket_status(
    periods: &[(i64, i64, Status)],
    from_ms: i64,
    bucket_ms: i64,
    count: usize,
) -> Vec<Option<Status>> {
    (0..count)
        .map(|i| {
            let a = from_ms + bucket_ms * i as i64;
            let b = a + bucket_ms;
            periods
                .iter()
                .filter(|(pa, pb, _)| *pa < b && *pb > a)
                .map(|(_, _, s)| *s)
                .max_by_key(|s| rank(*s))
        })
        .collect()
}

/// Lower and upper end of the scale: the range of the values, from 0 when the values are close to
/// it anyway (so the bars keep their proportions), else with some room below the lowest value (so a
/// change of a value far from 0, like a temperature, stays visible).
fn scale(values: &[f64]) -> Option<(f64, f64)> {
    let lo = values.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !lo.is_finite() || !hi.is_finite() {
        return None;
    }
    if hi <= lo {
        // A constant value: the bars stand at mid height.
        return Some((lo - 1.0, lo + 1.0));
    }
    if lo >= 0.0 && lo <= hi * 0.5 {
        return Some((0.0, hi));
    }
    Some((lo - (hi - lo) * 0.25, hi))
}

/// Bars for measured values (bucket averages, `None` = no value) and the worst status of each
/// bucket. Values beyond a limit are coloured like the limit; columns of an outage are tinted, and
/// without a value they show a thin line at the bottom.
pub fn value_bars(
    values: &[Option<f64>],
    statuses: &[Option<Status>],
    thresholds: &Thresholds,
    height: u32,
) -> Vec<Bar> {
    let finite: Vec<f64> = values
        .iter()
        .flatten()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    let range = scale(&finite);
    let top = f64::from(height.saturating_sub(2));
    values
        .iter()
        .enumerate()
        .map(|(i, value)| {
            let status = statuses.get(i).copied().flatten();
            let value = value.filter(|v| v.is_finite());
            let tint = match (status, value) {
                (Some(Status::Down), _) => Some(Tone::Down),
                (Some(Status::Warning), None) => Some(Tone::Warning),
                _ => None,
            };
            match (value, range) {
                (Some(v), Some((lo, hi))) => {
                    let h = 2.0 + ((v - lo) / (hi - lo)).clamp(0.0, 1.0) * top;
                    let (color, class) = match thresholds.breach(v) {
                        Some(b) if b.is_down() => (Tone::Down.band(), "band-err"),
                        Some(_) => (WARN_BAR, "band-warn"),
                        None => (SERIES, "bar"),
                    };
                    Bar {
                        height: h.round() as u32,
                        color,
                        class,
                        tint,
                    }
                }
                _ => match tint {
                    Some(tone) => Bar {
                        height: 3,
                        color: tone.band(),
                        class: band_class(tone),
                        tint,
                    },
                    None => Bar::EMPTY,
                },
            }
        })
        .collect()
}

fn band_class(tone: Tone) -> &'static str {
    match tone {
        Tone::Down => "band-err",
        Tone::Warning => "band-warn",
        Tone::Up => "band-ok",
        Tone::Off => "band-off",
    }
}

/// Bars for sensors whose value is a status (interface status, uptime): one full column per bucket
/// in the colour of its worst status.
pub fn status_bars(statuses: &[Option<Status>], height: u32) -> Vec<Bar> {
    statuses
        .iter()
        .map(|status| match status {
            Some(s) => {
                let tone = Tone::of(*s);
                Bar {
                    height,
                    color: tone.band(),
                    class: band_class(tone),
                    tint: None,
                }
            }
            None => Bar::EMPTY,
        })
        .collect()
}

/// Local times of a mail, in the time zone of the server (the one of the maintenance windows).
#[derive(Debug, Clone)]
pub struct Clock {
    lang: Lang,
    zone: TimeZone,
    now_ms: i64,
}

impl Clock {
    pub fn new(lang: Lang, zone: TimeZone, now_ms: i64) -> Self {
        Self { lang, zone, now_ms }
    }

    /// In the time zone of the server ([`crate::schedule::time_zone`]).
    pub fn local(lang: Lang, now_ms: i64) -> Self {
        Self::new(lang, crate::schedule::time_zone().clone(), now_ms)
    }

    pub fn now_ms(&self) -> i64 {
        self.now_ms
    }

    pub fn zone_name(&self) -> String {
        crate::schedule::zone_name(&self.zone)
    }

    fn zoned(&self, ms: i64) -> Option<jiff::Zoned> {
        Timestamp::from_millisecond(ms)
            .ok()
            .map(|ts| ts.to_zoned(self.zone.clone()))
    }

    /// `14:32`
    pub fn time(&self, ms: i64) -> String {
        self.zoned(ms)
            .map(|z| format!("{:02}:{:02}", z.hour(), z.minute()))
            .unwrap_or_default()
    }

    /// `19.11.2026` or `11/19/2026`
    pub fn date(&self, ms: i64) -> String {
        self.zoned(ms)
            .map(|z| {
                self.lang.fmt(
                    "mail-date",
                    &[
                        ("day", format!("{:02}", z.day()).into()),
                        ("month", format!("{:02}", z.month()).into()),
                        ("year", z.year().to_string().into()),
                    ],
                )
            })
            .unwrap_or_default()
    }

    /// `Do, 19.11.2026 · 14:32`
    /// `14.11.` (German), `11/14` (English)
    pub fn short_date(&self, ms: i64) -> String {
        self.zoned(ms)
            .map(|z| {
                self.lang.fmt(
                    "mail-date-short",
                    &[
                        ("day", format!("{:02}", z.day()).into()),
                        ("month", format!("{:02}", z.month()).into()),
                    ],
                )
            })
            .unwrap_or_default()
    }

    pub fn day_time(&self, ms: i64) -> String {
        let Some(z) = self.zoned(ms) else {
            return String::new();
        };
        let weekday = z.weekday().to_monday_one_offset();
        format!(
            "{}, {} · {}",
            self.lang.get(&format!("weekday-{weekday}")),
            self.date(ms),
            self.time(ms)
        )
    }

    /// The time alone on the day of the mail, else with the date.
    pub fn when(&self, ms: i64) -> String {
        let same_day = match (self.zoned(ms), self.zoned(self.now_ms)) {
            (Some(a), Some(b)) => a.date() == b.date(),
            _ => false,
        };
        if same_day {
            self.time(ms)
        } else {
            format!("{} {}", self.date(ms), self.time(ms))
        }
    }
}

/// Everything around the content of a mail: title, preview line, colour band, head and foot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub lang_code: &'static str,
    pub title: String,
    /// The line mail programs show next to the subject.
    pub preheader: String,
    /// Colour band at the top of the card.
    pub band: Option<Tone>,
    /// Host of the web interface, top right (empty = none).
    pub host: String,
    /// "Sent by opennetmon 0.7.0 to 2 recipients · times in Europe/Berlin".
    pub footer: Vec<String>,
    /// Link to the e-mail settings, if the address of the web interface is known.
    pub settings: Option<String>,
}

/// The usual lines of the foot: who sent it to how many recipients, and the time zone.
pub fn footer(t: Lang, recipients: usize, clock: &Clock) -> Vec<String> {
    vec![
        t.fmt(
            "mail-footer-sent",
            &[
                ("version", crate::web::VERSION.into()),
                ("count", recipients.into()),
            ],
        ),
        t.get1("mail-footer-zone", "zone", clock.zone_name()),
    ]
}

/// The last lines of the plain text: version and, with the address of the web interface, the link
/// to the settings.
pub fn text_footer(t: Lang, links: Option<&Links>) -> String {
    let mut out = format!("-- \nopennetmon {}", crate::web::VERSION);
    if let Some(links) = links {
        out.push_str(&format!(
            " · {}: {}",
            t.get("mail-footer-settings"),
            links.mail_settings()
        ));
    }
    out.push('\n');
    out
}

/// Plain-text lines `label: value` with the values aligned.
pub fn aligned(rows: &[(String, String)]) -> String {
    let width = rows
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0);
    rows.iter()
        .map(|(label, value)| {
            let pad = width - label.chars().count();
            format!("{label}:{} {value}\n", " ".repeat(pad))
        })
        .collect()
}

/// The HTML with the inline images as `data:` URIs instead of `cid:` references, so a browser can
/// show it. Only for previews during development: mails never contain `data:` images.
#[cfg(test)]
pub fn with_data_uris(html: &str, images: &[InlineImage]) -> String {
    use base64::Engine;
    let mut out = html.to_owned();
    for image in images {
        let data = base64::engine::general_purpose::STANDARD.encode(&image.data);
        out = out.replace(
            &format!("cid:{}", image.content_id),
            &format!("data:{};base64,{data}", image.content_type),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_keep_their_unit_and_durations_are_rounded() {
        assert_eq!(
            nb("über 90 °C, 3 Std. 5 Min."),
            "über 90\u{a0}°C, 3\u{a0}Std. 5\u{a0}Min."
        );
        assert_eq!(nb("Gerät A ist da"), "Gerät A ist da");
        let d = |secs: i64| duration(Lang::De, secs * 1_000);
        assert_eq!(d(45), "45 s");
        assert_eq!(d(2_520), "42 Min.");
        assert_eq!(d(7_390), "2 Std. 3 Min.");
        assert_eq!(d(3 * 86_400 + 3 * 3_600 + 59 * 60), "3 Tage 4 Std.");
        assert_eq!(d(86_400 + 600), "1 Tag");
        assert_eq!(
            duration(Lang::En, (2 * 86_400 + 3_600) * 1_000),
            "2 days 1 h"
        );
    }

    #[test]
    fn buckets_take_the_worst_status_of_their_time() {
        let periods = [
            (0, 150, Status::Up),
            (150, 200, Status::Down),
            (200, 400, Status::Up),
        ];
        assert_eq!(
            bucket_status(&periods, 0, 100, 5),
            [
                Some(Status::Up),
                Some(Status::Down),
                Some(Status::Up),
                Some(Status::Up),
                None
            ]
        );
    }

    #[test]
    fn values_near_zero_keep_their_proportions_others_show_their_change() {
        assert_eq!(scale(&[1.0, 2.0, 4.0]), Some((0.0, 4.0)));
        // A temperature between 60 and 80: starting at 0 would flatten the rise.
        assert_eq!(scale(&[60.0, 80.0]), Some((55.0, 80.0)));
        assert_eq!(scale(&[5.0, 5.0]), Some((4.0, 6.0)));
        assert_eq!(scale(&[]), None);
        assert_eq!(scale(&[-10.0, 10.0]), Some((-15.0, 10.0)));
    }

    #[test]
    fn bars_show_limits_and_outages() {
        let thresholds = Thresholds {
            warn_above: Some(75.0),
            down_above: Some(90.0),
            ..Thresholds::default()
        };
        let bars = value_bars(
            &[Some(60.0), Some(80.0), Some(95.0), None, None],
            &[
                Some(Status::Up),
                Some(Status::Warning),
                Some(Status::Down),
                Some(Status::Down),
                None,
            ],
            &thresholds,
            62,
        );
        // 60..95 is drawn from 51.25 (a quarter of the span below), so the lowest bar keeps some
        // height: 2 + 8.75 / 43.75 * 60.
        assert_eq!(
            bars[0].height, 14,
            "the lowest value stands a little above the bottom"
        );
        assert_eq!((bars[1].color, bars[1].class), (WARN_BAR, "band-warn"));
        assert_eq!(bars[1].tint, None);
        assert_eq!(
            (bars[2].height, bars[2].class, bars[2].tint),
            (62, "band-err", Some(Tone::Down))
        );
        assert_eq!((bars[3].height, bars[3].tint), (3, Some(Tone::Down)));
        assert_eq!(bars[4], Bar::EMPTY);
        let chart = Chart {
            bars: vec![Bar::EMPTY],
            height: 10,
            start: String::new(),
            end: String::new(),
            legend: Vec::new(),
        };
        assert!(!chart.has_bars());
    }

    #[test]
    fn status_sensors_get_coloured_columns() {
        let bars = status_bars(&[Some(Status::Up), None, Some(Status::Down)], 20);
        assert_eq!((bars[0].height, bars[0].class), (20, "band-ok"));
        assert_eq!(bars[1], Bar::EMPTY);
        assert_eq!(bars[2].color, Tone::Down.fg());
    }

    #[test]
    fn local_times_follow_the_zone_and_the_language() {
        let zone = TimeZone::get("Europe/Berlin").unwrap();
        // 2026-11-19 13:32:00 UTC, a Thursday: 14:32 in Berlin.
        let at = 1_795_095_120_000;
        let de = Clock::new(Lang::De, zone.clone(), at + 60_000);
        assert_eq!(de.day_time(at), "Do, 19.11.2026 · 14:32");
        assert_eq!(de.when(at), "14:32");
        assert_eq!(de.when(at - 86_400_000), "18.11.2026 14:32");
        assert_eq!(de.zone_name(), "Europe/Berlin");
        let en = Clock::new(Lang::En, zone, at);
        assert_eq!(en.day_time(at), "Thu, 11/19/2026 · 14:32");
    }

    #[test]
    fn links_are_built_from_the_address_and_ids() {
        let links = Links::new(Some("https://monitor.example.org:8443/onm")).unwrap();
        assert_eq!(links.host(), "monitor.example.org:8443");
        assert_eq!(
            links.sensor(12),
            "https://monitor.example.org:8443/onm/sensors/12"
        );
        assert_eq!(
            links.device(3),
            "https://monitor.example.org:8443/onm/devices/3/sensors"
        );
        assert_eq!(links.overview(), "https://monitor.example.org:8443/onm/");
        assert!(Links::new(None).is_none());
    }

    #[test]
    fn plain_text_values_line_up() {
        let rows = [
            ("Seit".to_owned(), "14:32".to_owned()),
            ("Verfügbarkeit".to_owned(), "99 %".to_owned()),
        ];
        assert_eq!(
            aligned(&rows),
            "Seit:          14:32\nVerfügbarkeit: 99 %\n"
        );
    }
}
