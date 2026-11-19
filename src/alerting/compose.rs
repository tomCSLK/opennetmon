//! The alert mail: subject, plain text and HTML for one change or a batch of changes, from the
//! changes and the details [`super::details`] gathered for them. Pure functions, in the mail
//! language; the templates are `templates/mail/alert.html` and `templates/mail/batch.html`.

use askama::Template;

use crate::i18n::Lang;
use crate::mail::view::{
    self, Bar, Button, Chart, Chip, Clock, Fact, Frame, Head, LegendItem, Links, Tone, duration,
    filters,
};
use crate::mail::{Mail, header_text};
use crate::status::Status;

use super::AlertEvent;

/// Changes a batch mail lists in detail; the rest is counted.
pub const MAX_ROWS: usize = 30;
/// Changes of a batch mail with a small history (the HTML must stay small: Gmail cuts mails off
/// at about 100 KB).
pub const MINI_ROWS: usize = 10;
/// Devices listed under "Impact".
const MAX_IMPACT_ROWS: usize = 10;
/// Longest message in a subject.
const SUBJECT_MESSAGE_CHARS: usize = 120;
/// Height of the history chart and of the small ones, in pixels.
pub const CHART_HEIGHT: u32 = 64;
pub const MINI_HEIGHT: u32 = 22;

/// A device whose sensors depend on the sensor of an outage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependent {
    pub name: String,
    pub host: String,
    pub sensors: usize,
}

/// The history of a sensor around a change, as bars.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    pub from_ms: i64,
    pub to_ms: i64,
    /// "Response time, last 6 hours".
    pub title: String,
    pub bars: Vec<Bar>,
    pub legend: Vec<LegendItem>,
    /// The history as a picture ([`crate::chart_png`]); without it the mail shows the bars.
    pub png: Option<Vec<u8>>,
    /// Name of the channel, for the alt text of the picture: "Antwortzeit".
    pub channel: String,
}

/// What a mail tells about a change beyond the change itself. Everything is optional: details
/// that could not be read are left out, the mail goes out anyway.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Details {
    pub group_path: Vec<String>,
    pub host: String,
    /// When the status the sensor left began (the change before this one).
    pub previous_ms: Option<i64>,
    /// The latest value of the main channel, formatted, and its time.
    pub value: Option<(String, i64)>,
    /// "Warning above 75 °C · Down above 90 °C" (empty = no limits).
    pub limits: String,
    /// Availability of the last 7 days, formatted.
    pub availability: Option<String>,
    /// Outages of the last 7 days: number and their total length, formatted.
    pub outages: Option<(i64, String)>,
    pub history: Option<History>,
    /// For an outage the devices it pauses through dependencies, for a recovery the devices that
    /// depend on it.
    pub dependents: Vec<Dependent>,
    /// Mean of the main channel over the history, formatted.
    pub average: Option<String>,
    /// The limit the latest value is beyond: "über 75 °C".
    pub beyond: Option<String>,
    /// Change over the last 2 hours and the range before it: ("+9 °C", "62–66 °C").
    pub trend: Option<(String, String)>,
    /// Highest (or, for a limit below, lowest) value of the 7 days before the change: catalog id of
    /// the label, the value and its time.
    pub peak: Option<(&'static str, String, i64)>,
    /// The sensor answers requests (ping, services): its recovery reads "answers again".
    pub answers: bool,
    /// Other sensors of the device (for a warning).
    pub others: Vec<Other>,
}

/// Another sensor of the same device, with its latest value and status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Other {
    pub name: String,
    pub value: String,
    pub status: Status,
}

/// A change with its details.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub event: AlertEvent,
    pub details: Details,
}

/// Where and for whom a mail is written.
pub struct Context {
    pub lang: Lang,
    pub links: Option<Links>,
    pub recipients: usize,
    pub clock: Clock,
    /// Seconds within which changes come in one mail (for the foot of a batch mail).
    pub batch_secs: u64,
}

/// The subject word for a single change.
fn word_id(status: Status) -> &'static str {
    match status {
        Status::Down => "alert-subject-down",
        Status::Warning => "alert-subject-warning",
        Status::Up => "alert-subject-up",
        Status::Unknown => "alert-subject-unknown",
        Status::Paused => "alert-subject-paused",
    }
}

/// The message of a change as one line.
fn message_of(t: Lang, event: &AlertEvent) -> String {
    header_text(&crate::status::shown(t, &event.change.message), 500)
}

/// "Switch Office · Ping"
fn title_of(event: &AlertEvent) -> String {
    format!("{} · {}", event.device, event.sensor)
}

/// How long the status before a recovery lasted, if known.
fn lasted(item: &Item) -> Option<i64> {
    item.details
        .previous_ms
        .map(|from| item.event.change.ts_ms - from)
        .filter(|ms| *ms >= 0)
}

fn lasted_id(from: Option<Status>, down: &'static str, warning: &'static str) -> &'static str {
    if from == Some(Status::Warning) {
        warning
    } else {
        down
    }
}

/// Subject and plain text plus HTML for one or more changes.
pub fn compose(ctx: &Context, items: &[Item]) -> Mail {
    let (subject, text, html, picture) = match items {
        [one] => single(ctx, one),
        many => {
            let (subject, text, html) = batch(ctx, many);
            (subject, text, html, None)
        }
    };
    let html = match html {
        Ok(html) => Some(html),
        Err(e) => {
            tracing::warn!(error = %e, "rendering the HTML of an alert mail failed");
            None
        }
    };
    let inline = match (&html, picture) {
        (None, _) => Vec::new(),
        (Some(_), None) => vec![view::logo()],
        (Some(_), Some(png)) => vec![view::logo(), view::chart_image(png)],
    };
    Mail {
        subject: header_text(&subject, 250),
        text,
        html,
        inline,
    }
}

fn chart(ctx: &Context, history: &History, height: u32) -> Chart {
    Chart {
        bars: history.bars.clone(),
        height,
        start: ctx.clock.time(history.from_ms),
        end: ctx.clock.time(history.to_ms),
        legend: history.legend.clone(),
    }
}

fn frame(ctx: &Context, title: String, preheader: String, band: Option<Tone>) -> Frame {
    Frame {
        lang_code: ctx.lang.code(),
        title,
        preheader,
        band,
        host: ctx.links.as_ref().map(Links::host).unwrap_or_default(),
        footer: view::footer(ctx.lang, ctx.recipients, &ctx.clock),
        settings: ctx.links.as_ref().map(Links::mail_settings),
    }
}

/// The section "Impact" below an outage or a recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Impact {
    intro: String,
    chip: Option<Chip>,
    rows: Vec<ImpactRow>,
    more: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImpactRow {
    name: String,
    host: String,
    count: String,
}

/// A row of "Other sensors of this device".
#[derive(Debug, Clone, PartialEq, Eq)]
struct OtherRow {
    name: String,
    value: String,
    chip: Chip,
}

#[derive(Template)]
#[template(path = "mail/alert.html")]
struct AlertTemplate {
    t: Lang,
    frame: Frame,
    head: Head,
    tone: Tone,
    message: String,
    facts: Vec<Fact>,
    chart: Option<Chart>,
    /// Alt text of the history picture, if the mail has one (then instead of `chart`).
    picture: Option<String>,
    chart_title: String,
    impact: Option<Impact>,
    others: Vec<OtherRow>,
    buttons: Vec<Button>,
}

fn sensors_text(t: Lang, count: usize) -> String {
    t.get1("mail-impact-sensors", "count", count)
}

/// The impact of a change, if it has one: what an outage pauses, what runs again after it.
fn impact(t: Lang, item: &Item) -> Option<Impact> {
    let deps = &item.details.dependents;
    let total: usize = deps.iter().map(|d| d.sensors).sum();
    match item.event.change.to {
        Status::Down => {
            let intro = if deps.is_empty() {
                let (sensors, devices) = item.event.paused_dependents?;
                t.get2(
                    "alert-dependents-paused",
                    "sensors",
                    sensors,
                    "devices",
                    devices,
                )
            } else {
                t.get("mail-impact-paused")
            };
            let rows = deps
                .iter()
                .take(MAX_IMPACT_ROWS)
                .map(|d| ImpactRow {
                    name: d.name.clone(),
                    host: d.host.clone(),
                    count: sensors_text(t, d.sensors),
                })
                .collect();
            let more = deps.len().saturating_sub(MAX_IMPACT_ROWS);
            Some(Impact {
                intro,
                chip: Some(Chip {
                    tone: Tone::Off,
                    text: t.get("status-paused"),
                }),
                rows,
                more: if more > 0 {
                    t.get1("mail-impact-more", "count", more)
                } else {
                    String::new()
                },
            })
        }
        Status::Up if total > 0 => {
            let names: Vec<&str> = deps
                .iter()
                .take(MAX_IMPACT_ROWS)
                .map(|d| d.name.as_str())
                .collect();
            let mut names = names.join(", ");
            if deps.len() > MAX_IMPACT_ROWS {
                names.push_str(", …");
            }
            Some(Impact {
                intro: t.fmt(
                    "mail-impact-resumed",
                    &[
                        ("sensors", total.into()),
                        ("devices", deps.len().into()),
                        ("names", names.into()),
                    ],
                ),
                chip: None,
                rows: Vec::new(),
                more: String::new(),
            })
        }
        _ => None,
    }
}

/// "vorher 3 Tage 3 Std. in Ordnung": how long the status before a change lasted.
fn before_text(t: Lang, from: Status, ms: i64) -> String {
    let id = match from {
        Status::Up => "mail-before-up",
        Status::Warning => "mail-before-warning",
        Status::Down => "mail-before-down",
        Status::Unknown => "mail-before-unknown",
        Status::Paused => "mail-before-paused",
    };
    t.get1(id, "duration", duration(t, ms))
}

/// The key figures of a single change.
fn facts(ctx: &Context, item: &Item) -> Vec<Fact> {
    let t = ctx.lang;
    let change = &item.event.change;
    let d = &item.details;
    let mut out = Vec::new();
    let at = |ts: i64| t.get1("mail-fact-at", "time", ctx.clock.when(ts));
    let since_fact = || {
        let before = match (change.from, d.previous_ms) {
            (Some(from), Some(since)) if since <= change.ts_ms => {
                before_text(t, from, change.ts_ms - since)
            }
            _ => String::new(),
        };
        Fact::new(
            t.get("mail-fact-since"),
            ctx.clock.when(change.ts_ms),
            before,
        )
    };
    let limits_fact = || {
        (!d.limits.is_empty())
            .then(|| Fact::new(t.get("mail-fact-limits"), d.limits.clone(), String::new()))
    };
    match change.to {
        Status::Up => {
            if let Some(ms) = lasted(item) {
                out.push(Fact::new(
                    t.get(lasted_id(
                        change.from,
                        "mail-fact-down-lasted",
                        "mail-fact-warning-lasted",
                    )),
                    duration(t, ms),
                    format!(
                        "{} – {}",
                        ctx.clock.when(change.ts_ms - ms),
                        ctx.clock.when(change.ts_ms)
                    ),
                ));
            }
            if let Some((value, ts)) = &d.value {
                let sub = match &d.average {
                    Some(avg) => t.get1("mail-fact-average", "value", avg.as_str()),
                    None => at(*ts),
                };
                out.push(Fact::new(t.get("mail-fact-value"), value.clone(), sub));
            }
            if let Some((count, total)) = &d.outages {
                out.push(Fact::new(
                    t.get("mail-fact-outages"),
                    count.to_string(),
                    if *count > 0 {
                        t.get1("mail-outages-total", "duration", total.as_str())
                    } else {
                        String::new()
                    },
                ));
            }
            if let Some(percent) = &d.availability {
                out.push(Fact::new(
                    t.get("mail-fact-availability"),
                    percent.clone(),
                    String::new(),
                ));
            }
        }
        Status::Warning => {
            match &d.value {
                Some((value, _)) => {
                    let since = ctx.clock.when(change.ts_ms);
                    let sub = match &d.beyond {
                        Some(limit) => {
                            t.get2("mail-since-beyond", "time", since, "limit", limit.as_str())
                        }
                        None => t.get1("mail-since-time", "time", since),
                    };
                    out.push(Fact::new(t.get("mail-fact-now"), value.clone(), sub));
                }
                None => out.push(since_fact()),
            }
            if let Some((delta, range)) = &d.trend {
                out.push(Fact::new(
                    t.get("mail-fact-trend"),
                    t.get1("mail-trend", "delta", delta.as_str()),
                    t.get1("mail-trend-before", "range", range.as_str()),
                ));
            }
            out.extend(limits_fact());
            if let Some((label, value, ts)) = &d.peak {
                out.push(Fact::new(
                    t.get(label),
                    value.clone(),
                    t.get2(
                        "mail-peak-at",
                        "date",
                        ctx.clock.short_date(*ts),
                        "time",
                        ctx.clock.time(*ts),
                    ),
                ));
            }
        }
        _ => {
            out.push(since_fact());
            if let Some((value, ts)) = &d.value {
                out.push(Fact::new(
                    t.get("mail-fact-last-value"),
                    value.clone(),
                    at(*ts),
                ));
            }
            out.extend(limits_fact());
            if let Some(percent) = &d.availability {
                out.push(Fact::new(
                    t.get("mail-fact-availability"),
                    percent.clone(),
                    d.outages
                        .as_ref()
                        .map(|(count, total)| {
                            if *count == 0 {
                                t.get("mail-outages-none")
                            } else {
                                t.get2("mail-outages", "count", count, "duration", total.as_str())
                            }
                        })
                        .unwrap_or_default(),
                ));
            }
        }
    }
    out
}

/// The text of the message box. A recovery whose message is only the generic "OK" says what the
/// sensor shows again: "Antwortet wieder – 1,4 ms".
fn shown_message(t: Lang, item: &Item) -> String {
    let message = message_of(t, &item.event);
    let generic =
        message.is_empty() || message == t.get("assess-ok") || message.eq_ignore_ascii_case("ok");
    match (&item.details.value, item.event.change.to) {
        (Some((value, _)), Status::Up) if generic => t.get1(
            if item.details.answers {
                "mail-up-answers"
            } else {
                "mail-up-value"
            },
            "value",
            value.as_str(),
        ),
        _ => message,
    }
}

/// The alt text of the history picture: what it shows and what happened in it.
fn picture_alt(ctx: &Context, item: &Item, history: &History) -> String {
    let t = ctx.lang;
    let change = &item.event.change;
    let channel = history.channel.as_str();
    let since = ctx.clock.when(change.ts_ms);
    match change.to {
        Status::Down => t.fmt(
            "mail-chart-alt-down",
            &[
                ("channel", channel.into()),
                ("time", since.into()),
                (
                    "state",
                    t.get(if item.details.answers {
                        "mail-chart-no-answer"
                    } else {
                        "mail-chart-outage"
                    })
                    .into(),
                ),
            ],
        ),
        Status::Up => match lasted(item) {
            Some(ms) => t.fmt(
                "mail-chart-alt-up",
                &[
                    ("channel", channel.into()),
                    ("from", ctx.clock.when(change.ts_ms - ms).into()),
                    ("to", since.into()),
                ],
            ),
            None => t.get1("mail-chart-alt", "channel", channel),
        },
        _ => match &item.details.beyond {
            Some(limit) => t.fmt(
                "mail-chart-alt-warning",
                &[
                    ("channel", channel.into()),
                    ("time", since.into()),
                    ("limit", limit.as_str().into()),
                ],
            ),
            None => t.get1("mail-chart-alt", "channel", channel),
        },
    }
}

/// Subject, plain text, HTML and the history picture (if the HTML shows it) of a single change.
type Single = (String, String, askama::Result<String>, Option<Vec<u8>>);

fn single(ctx: &Context, item: &Item) -> Single {
    let t = ctx.lang;
    let event = &item.event;
    let change = &event.change;
    let tone = Tone::of(change.to);
    let word = t.get(word_id(change.to));
    let title = title_of(event);
    let message = shown_message(t, item);
    let since = t.get1("mail-since-time", "time", ctx.clock.when(change.ts_ms));

    let subject_note = match (change.to, lasted(item)) {
        (Status::Up, Some(ms)) => format!(
            " ({})",
            t.get1(
                lasted_id(
                    change.from,
                    "mail-subject-down-lasted",
                    "mail-subject-warning-lasted"
                ),
                "duration",
                duration(t, ms)
            )
        ),
        _ if message.is_empty() => String::new(),
        _ => format!(" – {}", header_text(&message, SUBJECT_MESSAGE_CHARS)),
    };
    let subject = format!("[opennetmon] {word}: {title}{subject_note}");

    let mut preheader = match change.to {
        Status::Up => {
            let mut text = t.get1("mail-pre-up", "time", ctx.clock.when(change.ts_ms));
            if let Some(ms) = lasted(item) {
                text.push(' ');
                text.push_str(&t.get1(
                    lasted_id(
                        change.from,
                        "mail-pre-down-lasted",
                        "mail-pre-warning-lasted",
                    ),
                    "duration",
                    duration(t, ms),
                ));
            }
            text
        }
        _ if message.is_empty() => format!("{since}."),
        _ => format!("{message}, {since}."),
    };
    if let (Status::Down, Some((sensors, devices))) = (change.to, event.paused_dependents) {
        preheader.push(' ');
        preheader.push_str(&t.get2(
            "alert-dependents-paused",
            "sensors",
            sensors,
            "devices",
            devices,
        ));
        preheader.push('.');
    }

    let facts = facts(ctx, item);
    let impact = impact(t, item);
    let mut buttons = Vec::new();
    if let Some(links) = &ctx.links {
        let id = change.sensor_id;
        buttons.push(Button {
            label: t.get("mail-open-sensor"),
            href: links.sensor(id),
            primary: true,
        });
        buttons.push(Button {
            label: t.get("mail-open-device"),
            href: links.device(event.device_id),
            primary: false,
        });
        let (label, href) = match change.to {
            Status::Down => ("mail-open-log", links.sensor_log(id)),
            Status::Warning => ("mail-open-thresholds", links.sensor_settings(id)),
            _ => ("mail-open-report", links.report_week()),
        };
        buttons.push(Button {
            label: t.get(label),
            href,
            primary: false,
        });
    }

    // Plain text.
    let zone = ctx.clock.zone_name();
    let mut text = format!(
        "{}  {title}\n{} ({zone})\n\n",
        word.to_uppercase(),
        ctx.clock.day_time(change.ts_ms)
    );
    if !message.is_empty() {
        text.push_str(&message);
        text.push_str("\n\n");
    }
    let mut rows: Vec<(String, String)> = facts
        .iter()
        .map(|f| {
            let value = if f.sub.is_empty() {
                f.value.clone()
            } else {
                format!("{} ({})", f.value, f.sub)
            };
            (f.label.clone(), value)
        })
        .collect();
    let mut device = item.details.group_path.clone();
    device.push(event.device.clone());
    let mut device = device.join(" › ");
    if !item.details.host.is_empty() {
        device.push_str(&format!(" ({})", item.details.host));
    }
    rows.push((t.get("mail-fact-device"), device));
    text.push_str(&view::aligned(&rows));
    if let Some(impact) = &impact {
        text.push('\n');
        // An outage names what it paused in numbers first, then device by device.
        let total: usize = item.details.dependents.iter().map(|d| d.sensors).sum();
        let counted = event
            .paused_dependents
            .or_else(|| (total > 0).then_some((total, item.details.dependents.len())));
        match (change.to, counted) {
            (Status::Down, Some((sensors, devices))) => text.push_str(&t.get2(
                "alert-dependents-paused",
                "sensors",
                sensors,
                "devices",
                devices,
            )),
            _ => text.push_str(&impact.intro),
        }
        text.push('\n');
        for row in &impact.rows {
            let host = if row.host.is_empty() {
                String::new()
            } else {
                format!(" ({})", row.host)
            };
            text.push_str(&format!("  - {}{host}, {}\n", row.name, row.count));
        }
        if !impact.more.is_empty() {
            text.push_str(&format!("  {}\n", impact.more));
        }
    }
    if !buttons.is_empty() {
        text.push('\n');
        text.push_str(&view::aligned(
            &buttons
                .iter()
                .map(|b| (b.label.clone(), b.href.clone()))
                .collect::<Vec<_>>(),
        ));
    }
    text.push('\n');
    text.push_str(&view::text_footer(t, ctx.links.as_ref()));

    // The picture if there is one, else the bars.
    let picture = item
        .details
        .history
        .as_ref()
        .and_then(|h| h.png.clone().map(|png| (picture_alt(ctx, item, h), png)));
    let chart = item
        .details
        .history
        .as_ref()
        .filter(|_| picture.is_none())
        .map(|h| chart(ctx, h, CHART_HEIGHT))
        .filter(Chart::has_bars);
    let (picture_alt, png) = picture.unzip();
    let page = AlertTemplate {
        picture: picture_alt,
        t,
        frame: frame(ctx, word.clone(), preheader, Some(tone)),
        head: Head {
            chip: Chip { tone, text: word },
            when: ctx.clock.day_time(change.ts_ms),
            title,
            path: item.details.group_path.clone(),
            host: item.details.host.clone(),
            note: Vec::new(),
        },
        tone,
        message,
        facts,
        chart_title: item
            .details
            .history
            .as_ref()
            .map(|h| h.title.clone())
            .unwrap_or_default(),
        chart,
        impact,
        others: item
            .details
            .others
            .iter()
            .map(|o| OtherRow {
                name: o.name.clone(),
                value: o.value.clone(),
                chip: Chip {
                    tone: Tone::of(o.status),
                    text: t.get(o.status.label_id()),
                },
            })
            .collect(),
        buttons,
    };
    let html = page.render();
    let png = png.filter(|_| html.is_ok());
    (subject, text, html, png)
}

/// A counter at the top of a batch mail.
struct Counter {
    tone: Tone,
    count: usize,
    word: String,
}

struct Cause {
    title: String,
    text: String,
}

struct Row {
    chip: Chip,
    title: String,
    message: String,
    when: String,
    chart: Option<Chart>,
}

#[derive(Template)]
#[template(path = "mail/batch.html")]
struct BatchTemplate {
    t: Lang,
    frame: Frame,
    title: String,
    subtitle: String,
    counters: Vec<Counter>,
    cause: Option<Cause>,
    rows: Vec<Row>,
    more: String,
    buttons: Vec<Button>,
}

/// Worst first: outages, warnings, recoveries.
pub(super) fn order(status: Status) -> u8 {
    match status {
        Status::Down => 0,
        Status::Warning => 1,
        Status::Up => 3,
        Status::Unknown | Status::Paused => 2,
    }
}

fn batch(ctx: &Context, items: &[Item]) -> (String, String, askama::Result<String>) {
    let t = ctx.lang;
    let count = |status: Status| items.iter().filter(|i| i.event.change.to == status).count();
    let kinds = [
        (Status::Down, "alert-count-down", "mail-counter-down"),
        (
            Status::Warning,
            "alert-count-warning",
            "mail-counter-warning",
        ),
        (Status::Up, "alert-count-up", "mail-counter-up"),
    ];
    let parts: Vec<String> = kinds
        .iter()
        .filter(|(s, _, _)| count(*s) > 0)
        .map(|(s, id, _)| t.get1(id, "count", count(*s)))
        .collect();
    let subject = format!(
        "[opennetmon] {}",
        t.get2(
            "alert-subject-many",
            "count",
            items.len(),
            "parts",
            parts.join(", ")
        )
    );
    let counters: Vec<Counter> = kinds
        .iter()
        .filter(|(s, _, _)| count(*s) > 0)
        .map(|(s, _, word)| Counter {
            tone: Tone::of(*s),
            count: count(*s),
            word: t.get1(word, "count", count(*s)),
        })
        .collect();

    let mut sorted: Vec<&Item> = items.iter().collect();
    sorted.sort_by_key(|i| (order(i.event.change.to), i.event.change.ts_ms));
    let first = items
        .iter()
        .map(|i| i.event.change.ts_ms)
        .min()
        .unwrap_or(0);
    let last = items
        .iter()
        .map(|i| i.event.change.ts_ms)
        .max()
        .unwrap_or(0);
    let mut span = ctx.clock.day_time(first);
    if ctx.clock.time(last) != ctx.clock.time(first) {
        span.push_str(&format!(" – {}", ctx.clock.time(last)));
    }
    let title = t.get1("mail-batch-title", "count", items.len());

    // The outage that paused the most, if any: probably what the rest follows from.
    let cause = items
        .iter()
        .filter(|i| i.event.change.to == Status::Down)
        .filter_map(|i| i.event.paused_dependents.map(|p| (i, p)))
        .filter(|(_, (sensors, _))| *sensors > 0)
        .max_by_key(|(i, (sensors, _))| (*sensors, std::cmp::Reverse(i.event.change.ts_ms)))
        .map(|(i, (sensors, devices))| {
            let mut text = message_of(t, &i.event);
            if !text.is_empty() {
                text.push_str(", ");
            }
            text.push_str(&t.get1(
                "mail-since-time",
                "time",
                ctx.clock.when(i.event.change.ts_ms),
            ));
            text.push_str(". ");
            text.push_str(&t.get2(
                "alert-dependents-paused",
                "sensors",
                sensors,
                "devices",
                devices,
            ));
            text.push_str(". ");
            text.push_str(&t.get("mail-cause-silent"));
            Cause {
                title: title_of(&i.event),
                text,
            }
        });
    let preheader = match &cause {
        Some(c) => format!("{}: {} – {}", t.get("mail-cause"), c.title, c.text),
        None => parts.join(", "),
    };

    let rows: Vec<Row> = sorted
        .iter()
        .take(MAX_ROWS)
        .enumerate()
        .map(|(n, i)| Row {
            chip: Chip {
                tone: Tone::of(i.event.change.to),
                text: t.get(i.event.change.to.label_id()),
            },
            title: title_of(&i.event),
            message: message_of(t, &i.event),
            when: ctx.clock.when(i.event.change.ts_ms),
            chart: i
                .details
                .history
                .as_ref()
                .filter(|_| n < MINI_ROWS)
                .map(|h| chart(ctx, h, MINI_HEIGHT))
                .filter(Chart::has_bars),
        })
        .collect();
    let more = match items.len().saturating_sub(MAX_ROWS) {
        0 => String::new(),
        n => t.get1("mail-more-changes", "count", n),
    };
    let mut buttons = Vec::new();
    if let Some(links) = &ctx.links {
        buttons.push(Button {
            label: t.get("mail-open-overview"),
            href: links.overview(),
            primary: true,
        });
        buttons.push(Button {
            label: t.get("mail-open-log"),
            href: links.log(),
            primary: false,
        });
    }
    let band = counters.first().map(|c| c.tone);

    // Plain text.
    let mut text = format!(
        "{}  {span} ({})\n{}\n\n",
        title.to_uppercase(),
        ctx.clock.zone_name(),
        parts.join(", ")
    );
    if let Some(c) = &cause {
        text.push_str(&format!(
            "{}: {} – {}\n\n",
            t.get("mail-cause"),
            c.title,
            c.text
        ));
    }
    let words: Vec<String> = rows.iter().map(|r| r.chip.text.to_uppercase()).collect();
    let width = words.iter().map(|w| w.chars().count()).max().unwrap_or(0);
    for (row, word) in rows.iter().zip(&words) {
        let pad = " ".repeat(width - word.chars().count());
        let message = if row.message.is_empty() {
            String::new()
        } else {
            format!(" – {}", row.message)
        };
        text.push_str(&format!(
            "{word}{pad}  {}  {}{message}\n",
            row.when, row.title
        ));
    }
    if !more.is_empty() {
        text.push_str(&more);
        text.push('\n');
    }
    if !buttons.is_empty() {
        text.push('\n');
        text.push_str(&view::aligned(
            &buttons
                .iter()
                .map(|b| (b.label.clone(), b.href.clone()))
                .collect::<Vec<_>>(),
        ));
    }
    text.push('\n');
    text.push_str(&view::text_footer(t, ctx.links.as_ref()));

    let mut frame = frame(ctx, title.clone(), preheader, band);
    frame
        .footer
        .push(t.get1("mail-footer-batch", "seconds", ctx.batch_secs));
    let page = BatchTemplate {
        t,
        frame,
        title,
        subtitle: format!("{span} · {}", t.get("mail-batch-note")),
        counters,
        cause,
        rows,
        more,
        buttons,
    };
    (subject, text, page.render())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::i18n::Msg;
    use crate::status::Thresholds;
    use crate::storage::status::StatusChange;

    /// 2026-11-19 13:32 UTC, 14:32 in Berlin.
    pub(crate) const NOW: i64 = 1_795_095_120_000;
    const MINUTE: i64 = 60_000;

    pub(crate) fn context(lang: Lang, ui_url: Option<&str>) -> Context {
        Context {
            lang,
            links: Links::new(ui_url),
            recipients: 2,
            clock: Clock::new(lang, jiff::tz::TimeZone::get("Europe/Berlin").unwrap(), NOW),
            batch_secs: 10,
        }
    }

    fn event(
        sensor_id: i64,
        device: &str,
        sensor: &str,
        from: Status,
        to: Status,
        message: &str,
        ts_ms: i64,
    ) -> AlertEvent {
        AlertEvent {
            change: StatusChange {
                sensor_id,
                from: Some(from),
                to,
                message: Msg::Text(message.to_owned()),
                ts_ms,
            },
            device_id: 3,
            device: device.to_owned(),
            sensor: sensor.to_owned(),
            paused_dependents: None,
        }
    }

    /// A 6-hour history of 72 bars: a calm series, the last `bad` buckets in `status`.
    fn history(
        lang: Lang,
        base: f64,
        thresholds: &Thresholds,
        bad: usize,
        status: Status,
        values_while_bad: bool,
    ) -> History {
        sized_history(
            lang,
            base,
            thresholds,
            bad,
            status,
            values_while_bad,
            72,
            CHART_HEIGHT,
        )
    }

    /// The same with `count` bars of at most `height` pixels.
    #[allow(clippy::too_many_arguments)]
    fn sized_history(
        lang: Lang,
        base: f64,
        thresholds: &Thresholds,
        bad: usize,
        status: Status,
        values_while_bad: bool,
        count: usize,
        height: u32,
    ) -> History {
        let values: Vec<Option<f64>> = (0..count)
            .map(|i| {
                let wave = (i as f64 / 6.0).sin() * base * 0.2;
                let rising = if i + bad >= count {
                    base * 0.6 * (i + bad + 1 - count) as f64
                } else {
                    0.0
                };
                (i + bad < count || values_while_bad).then_some(base + wave + rising)
            })
            .collect();
        let statuses: Vec<Option<Status>> = (0..count)
            .map(|i| Some(if i + bad >= count { status } else { Status::Up }))
            .collect();
        let legend = if thresholds.warn_above.is_some() {
            vec![
                LegendItem {
                    text: t_get(lang, "Warnung über 75 °C", "Warning above 75 °C"),
                    tone: Some(Tone::Warning),
                },
                LegendItem {
                    text: t_get(lang, "Ausfall über 90 °C", "Down above 90 °C"),
                    tone: Some(Tone::Down),
                },
            ]
        } else {
            vec![LegendItem {
                text: "Max. 3,1 ms".to_owned(),
                tone: None,
            }]
        };
        History {
            from_ms: NOW - 6 * 60 * MINUTE,
            to_ms: NOW,
            title: t_get(
                lang,
                "Antwortzeit, letzte 6 Stunden",
                "Response time, last 6 hours",
            ),
            bars: view::value_bars(&values, &statuses, thresholds, height),
            legend,
            png: None,
            channel: t_get(lang, "Antwortzeit", "Response time"),
        }
    }

    fn t_get(lang: Lang, de: &str, en: &str) -> String {
        if lang == Lang::De { de } else { en }.to_owned()
    }

    pub(crate) fn outage(lang: Lang) -> Item {
        let mut e = event(
            7,
            "Switch Büro",
            "Ping",
            Status::Up,
            Status::Down,
            "Keine Antwort (Zeitüberschreitung nach 3 Versuchen)",
            NOW - MINUTE,
        );
        e.paused_dependents = Some((5, 2));
        Item {
            event: e,
            details: Details {
                group_path: vec!["Haus".to_owned(), "Netzwerk".to_owned()],
                host: "192.0.2.10".to_owned(),
                previous_ms: Some(NOW - 3 * 24 * 60 * MINUTE - 3 * 60 * MINUTE - MINUTE),
                value: Some(("1,4 ms".to_owned(), NOW - 2 * MINUTE)),
                limits: t_get(lang, "Warnung über 20 ms", "Warning above 20 ms"),
                availability: Some("99,82 %".to_owned()),
                outages: Some((2, "14 Min.".to_owned())),
                history: Some(history(
                    lang,
                    1.5,
                    &Thresholds::default(),
                    2,
                    Status::Down,
                    false,
                )),
                dependents: vec![
                    Dependent {
                        name: "AP Flur".to_owned(),
                        host: "192.0.2.21".to_owned(),
                        sensors: 3,
                    },
                    Dependent {
                        name: "Drucker".to_owned(),
                        host: "192.0.2.30".to_owned(),
                        sensors: 2,
                    },
                ],
                average: Some("1,6 ms".to_owned()),
                answers: true,
                ..Details::default()
            },
        }
    }

    /// The outage above, over after 42 minutes: the buckets of the outage have no values and are
    /// tinted.
    pub(crate) fn recovery(lang: Lang) -> Item {
        let mut item = outage(lang);
        item.event.change.from = Some(Status::Down);
        item.event.change.to = Status::Up;
        item.event.change.message = Msg::Text("OK".to_owned());
        item.event.paused_dependents = None;
        item.details.previous_ms = Some(NOW - MINUTE - 42 * MINUTE);
        item.details.outages = Some((3, "56 Min.".to_owned()));
        item.details.average = Some("1,6 ms".to_owned());
        let down = 63..72 - 1;
        let values: Vec<Option<f64>> = (0..72)
            .map(|i| (!down.contains(&i)).then_some(1.5 + (i as f64 / 6.0).sin() * 0.3))
            .collect();
        let statuses: Vec<Option<Status>> = (0..72)
            .map(|i| {
                Some(if down.contains(&i) {
                    Status::Down
                } else {
                    Status::Up
                })
            })
            .collect();
        let mut history = history(lang, 1.5, &Thresholds::default(), 0, Status::Up, true);
        history.bars = view::value_bars(&values, &statuses, &Thresholds::default(), CHART_HEIGHT);
        item.details.history = Some(history);
        item
    }

    /// A CPU temperature rising over the warning limit in the last 2 hours.
    pub(crate) fn warning(lang: Lang) -> Item {
        let thresholds = Thresholds {
            warn_above: Some(75.0),
            down_above: Some(90.0),
            ..Thresholds::default()
        };
        let values: Vec<Option<f64>> = (0..72)
            .map(|i| {
                let calm = 64.0 + (i as f64 / 5.0).sin() * 2.0;
                Some(if i < 48 {
                    calm
                } else {
                    calm + (i - 48) as f64 * 0.6
                })
            })
            .collect();
        let statuses: Vec<Option<Status>> = (0..72)
            .map(|i| Some(if i >= 69 { Status::Warning } else { Status::Up }))
            .collect();
        let mut history = history(lang, 60.0, &thresholds, 3, Status::Warning, true);
        history.bars = view::value_bars(&values, &statuses, &thresholds, CHART_HEIGHT);
        history.title = t_get(
            lang,
            "Temperatur, letzte 6 Stunden",
            "Temperature, last 6 hours",
        );
        history.channel = t_get(lang, "Temperatur", "Temperature");
        let other = |name: &str, value: &str, status| Other {
            name: name.to_owned(),
            value: value.to_owned(),
            status,
        };
        Item {
            event: event(
                12,
                "Server Keller",
                "CPU-Temperatur",
                Status::Up,
                Status::Warning,
                "78 °C über dem Grenzwert 75 °C",
                NOW - 12 * MINUTE,
            ),
            details: Details {
                group_path: vec!["Haus".to_owned(), "Server".to_owned()],
                host: "192.0.2.40".to_owned(),
                previous_ms: Some(NOW - 9 * 24 * 60 * MINUTE),
                value: Some(("78 °C".to_owned(), NOW - MINUTE)),
                limits: t_get(
                    lang,
                    "Warnung über 75 °C · Ausfall über 90 °C",
                    "Warning above 75 °C · Down above 90 °C",
                ),
                availability: Some("100 %".to_owned()),
                outages: Some((0, "0 s".to_owned())),
                history: Some(history),
                average: Some("68 °C".to_owned()),
                beyond: Some(t_get(lang, "über 75 °C", "above 75 °C")),
                trend: Some(("+9 °C".to_owned(), "62–66 °C".to_owned())),
                peak: Some((
                    "mail-fact-peak",
                    "71 °C".to_owned(),
                    NOW - 5 * 24 * 60 * MINUTE - 650 * MINUTE,
                )),
                others: vec![
                    other("Lüfter", "1 840 U/min", Status::Up),
                    other("Temperatur Datenträger", "47 °C", Status::Up),
                    other("CPU-Last", "92 %", Status::Warning),
                ],
                ..Details::default()
            },
        }
    }

    /// `count` changes as they come when a router fails: its outage first, then what is behind it.
    pub(crate) fn burst(lang: Lang, count: usize) -> Vec<Item> {
        let mut items = vec![outage(lang)];
        items[0].event.paused_dependents = Some((14, 3));
        items[0].details.history = Some(sized_history(
            lang,
            1.5,
            &Thresholds::default(),
            1,
            Status::Down,
            false,
            24,
            MINI_HEIGHT,
        ));
        for n in 1..count {
            let (from, to, message) = match n % 3 {
                0 => (Status::Up, Status::Warning, "Antwortzeit 48 ms über 20 ms"),
                1 => (Status::Up, Status::Down, "Keine Antwort"),
                _ => (Status::Down, Status::Up, "OK"),
            };
            let mut item = Item {
                event: event(
                    100 + n as i64,
                    &format!("Gerät {n}"),
                    "Port 3",
                    from,
                    to,
                    message,
                    NOW - MINUTE + n as i64 * 200,
                ),
                details: Details::default(),
            };
            if n < MINI_ROWS {
                let limits = Thresholds {
                    warn_above: Some(20.0),
                    ..Thresholds::default()
                };
                item.details.history = Some(sized_history(
                    lang,
                    if to == Status::Warning { 12.0 } else { 1.5 },
                    &limits,
                    1,
                    to,
                    to != Status::Down,
                    24,
                    MINI_HEIGHT,
                ));
            }
            items.push(item);
        }
        items
    }

    /// The HTML part with no-break spaces as plain spaces, for comparing texts.
    fn html(mail: &Mail) -> String {
        mail.html
            .as_deref()
            .expect("an HTML part")
            .replace('\u{a0}', " ")
    }

    /// Every `src` points into the mail, nothing is loaded from outside.
    fn assert_self_contained(html: &str) {
        for (i, _) in html.match_indices("src=\"") {
            assert!(
                html[i + 5..].starts_with("cid:"),
                "external source at {}",
                &html[i..(i + 60).min(html.len())]
            );
        }
        for bad in ["<link", "<script", "url(", "@import", "data:"] {
            assert!(!html.contains(bad), "{bad}");
        }
    }

    #[test]
    fn an_outage_says_what_where_since_when_and_what_it_pauses() {
        let mail = compose(
            &context(Lang::De, Some("https://onm.example.org")),
            &[outage(Lang::De)],
        );
        assert_eq!(
            mail.subject,
            "[opennetmon] Ausfall: Switch Büro · Ping – Keine Antwort (Zeitüberschreitung nach 3 Versuchen)"
        );
        assert_eq!(mail.inline, [view::logo()]);
        let html = html(&mail);
        assert_self_contained(&html);
        for part in [html.as_str(), mail.text.as_str()] {
            for expected in [
                "Switch Büro · Ping",
                "Haus",
                "Netzwerk",
                "192.0.2.10",
                "14:31",
                "Europe/Berlin",
                "1,4 ms",
                "Warnung über 20 ms",
                "99,82 %",
                "2 Ausfälle, zusammen 14 Min.",
                "AP Flur",
                "3 Sensoren",
                "https://onm.example.org/sensors/7",
                "https://onm.example.org/devices/3/sensors",
                "https://onm.example.org/log?sensor=7",
                "https://onm.example.org/settings/mail",
            ] {
                assert!(part.contains(expected), "{expected} missing in {part}");
            }
        }
        assert!(html.contains("<html lang=\"de\">"));
        assert!(html.contains("Antwortzeit, letzte 6 Stunden"));
        assert!(
            html.contains("Haus <span class=\"t3\" style=\"color:#636c78\">›</span> Netzwerk · <span class=\"mono\""),
            "› between groups, · before the address"
        );
        assert!(
            mail.text.starts_with(
                "AUSFALL  Switch Büro · Ping\nDo, 19.11.2026 · 14:31 (Europe/Berlin)\n"
            ),
            "{}",
            mail.text
        );
        assert!(
            mail.text
                .contains("Dadurch pausiert: 5 Sensoren an 2 Geräten")
        );
        assert!(
            mail.text.contains("(vorher 3 Tage 3 Std. in Ordnung)"),
            "{}",
            mail.text
        );
    }

    #[test]
    fn without_a_ui_address_there_are_no_links() {
        for lang in [Lang::De, Lang::En] {
            for items in [
                vec![outage(lang)],
                vec![recovery(lang)],
                vec![warning(lang)],
                burst(lang, 5),
            ] {
                let mail = compose(&context(lang, None), &items);
                let html = html(&mail);
                assert!(!html.contains("href="), "{html}");
                assert!(!html.contains("http") && !mail.text.contains("http"));
                assert_self_contained(&html);
            }
        }
    }

    #[test]
    fn a_recovery_names_the_length_of_the_outage_and_what_runs_again() {
        let mail = compose(
            &context(Lang::De, Some("https://onm.example.org/")),
            &[recovery(Lang::De)],
        );
        assert_eq!(
            mail.subject,
            "[opennetmon] Wieder in Ordnung: Switch Büro · Ping (Ausfall 42 Min.)"
        );
        let html = html(&mail);
        assert!(html.contains("Ausfall dauerte"));
        assert!(html.contains(
            "Die 5 pausierten Sensoren auf 2 Geräten (AP Flur, Drucker) laufen wieder; sie werden ab jetzt normal abgefragt."
        ));
        assert!(
            html.contains("Antwortet wieder – 1,4 ms"),
            "the generic OK says more"
        );
        assert!(mail.text.contains("Antwortet wieder – 1,4 ms"));
        assert!(html.contains("Durchschnitt 6 h: 1,6 ms"));
        assert!(
            mail.html.as_deref().unwrap().contains("class=\"err-bg\""),
            "the outage that just ended is tinted"
        );
        assert!(html.contains("https://onm.example.org/reports?period=7d&#38;view=sensors"));
        assert!(
            mail.text
                .contains("https://onm.example.org/reports?period=7d&view=sensors")
        );

        let mail = compose(&context(Lang::En, None), &[recovery(Lang::En)]);
        assert_eq!(
            mail.subject,
            "[opennetmon] OK again: Switch Büro · Ping (down for 42 min)"
        );
        assert!(mail.text.starts_with("OK AGAIN  "), "{}", mail.text);
        assert!(html_of(&mail).contains("<html lang=\"en\">"));
    }

    fn html_of(mail: &Mail) -> String {
        html(mail)
    }

    #[test]
    fn a_warning_links_to_the_limits() {
        let mail = compose(
            &context(Lang::De, Some("https://onm.example.org")),
            &[warning(Lang::De)],
        );
        assert!(
            mail.subject
                .starts_with("[opennetmon] Warnung: Server Keller · CPU-Temperatur – 78 °C")
        );
        let html = html(&mail);
        for expected in [
            "Aktuell",
            "seit 14:20 über 75 °C",
            "Tendenz",
            "+9 °C in 2 Std.",
            "vorher 62–66 °C",
            "Höchstwert 7 Tage",
            "am 14.11. um 03:42",
            "Weitere Kanäle dieses Geräts",
            "Temperatur Datenträger",
            "1 840 U/min",
            "CPU-Last",
        ] {
            assert!(html.contains(expected), "{expected}");
        }
        assert!(
            !html.contains("Verfügbarkeit"),
            "no availability in a warning"
        );
        // Number and unit never wrap apart.
        let raw = mail.html.as_deref().unwrap();
        assert!(raw.contains("90\u{a0}°C") && raw.contains("1\u{a0}840\u{a0}U/min"));
        assert!(
            !mail.text.contains('\u{a0}'),
            "the plain text keeps normal spaces"
        );
        assert!(html.contains("https://onm.example.org/sensors/12/edit"));
        assert!(html.contains("Grenzwerte ändern"));
        assert!(html.contains("Warnung über 75 °C · Ausfall über 90 °C"));
        assert!(
            !html.contains("Auswirkung"),
            "a warning has no impact section"
        );
    }

    #[test]
    fn foreign_text_is_escaped_and_headers_stay_single_lines() {
        let mut item = outage(Lang::De);
        item.event.device = "<b>x</b>&".to_owned();
        item.event.sensor = "Ping\r\nBcc: evil@example.org".to_owned();
        item.event.change.message =
            Msg::Text("<img src=\"http://evil.example/x.png\">\nZeile 2".to_owned());
        item.details.group_path = vec!["<script>alert(1)</script>".to_owned()];
        item.details.dependents[0].name = "\"><a href=\"http://evil.example\">".to_owned();
        let mail = compose(&context(Lang::De, None), &[item]);
        assert!(!mail.subject.contains('\r') && !mail.subject.contains('\n'));
        assert!(
            mail.subject
                .contains("<b>x</b>& · Ping Bcc: evil@example.org"),
            "{}",
            mail.subject
        );
        let html = html(&mail);
        assert!(html.contains("&#60;b&#62;x&#60;/b&#62;&#38;"), "{html}");
        assert!(!html.contains("<b>x</b>"));
        assert!(!html.contains("<script>alert"));
        assert!(!html.contains("<img src=\"http"));
        assert!(!html.contains("<a href=\"http://evil"));
        assert_self_contained(&html);
        // The plain text keeps the text as it is.
        assert!(mail.text.contains("<b>x</b>&"));
    }

    #[test]
    fn a_burst_is_one_summary_with_its_probable_cause() {
        let items = burst(Lang::De, 6);
        let mail = compose(&context(Lang::De, Some("https://onm.example.org")), &items);
        assert_eq!(
            mail.subject,
            "[opennetmon] 6 Statuswechsel: 3 Ausfälle, 1 Warnung, 2 wieder in Ordnung"
        );
        let html = html(&mail);
        assert_self_contained(&html);
        assert!(html.contains("Vermutliche Ursache"));
        assert!(html.contains("Dadurch pausiert: 14 Sensoren an 3 Geräten"));
        assert!(
            html.contains("https://onm.example.org/\""),
            "overview button"
        );
        assert!(html.contains("Wechsel innerhalb von 10 s kommen in einer E-Mail"));
        // Worst first: the outages before the warning before the recoveries.
        let down = mail.text.find("Gerät 1 ·").unwrap();
        let warn = mail.text.find("Gerät 3 ·").unwrap();
        let up = mail.text.find("Gerät 2 ·").unwrap();
        assert!(down < warn && warn < up, "{}", mail.text);
        assert!(
            mail.text
                .starts_with("6 STATUSWECHSEL  Do, 19.11.2026 · 14:31 (Europe/Berlin)\n"),
            "{}",
            mail.text
        );

        // Without an outage that paused anything, there is no cause.
        let mut calm = burst(Lang::En, 3);
        calm[0].event.paused_dependents = None;
        let mail = compose(&context(Lang::En, None), &calm);
        assert!(!html_of(&mail).contains("Probable cause"));
        assert_eq!(
            mail.subject,
            "[opennetmon] 3 status changes: 2 down, 1 OK again"
        );
    }

    #[test]
    fn a_long_burst_is_capped_and_stays_small() {
        let items = burst(Lang::De, MAX_ROWS + 40);
        let mail = compose(&context(Lang::De, Some("https://onm.example.org")), &items);
        let html = html(&mail);
        assert!(html.contains("… und 40 weitere"));
        assert!(mail.text.contains("… und 40 weitere"));
        // The outage that started it plus 29 others.
        assert_eq!(mail.text.matches(" · Port 3").count(), MAX_ROWS - 1);
        assert!(
            html.len() < 90_000,
            "{} bytes: Gmail cuts mails off at about 100 KB",
            html.len()
        );
    }

    /// The sample with a real history picture, drawn from data like the sample's (one value a
    /// minute over 6 hours).
    pub(crate) fn pictured(mut item: Item) -> Item {
        use crate::chart_png::{Band, ChartSpec, Colour, Threshold, render_png};
        let from = NOW - 6 * 60 * MINUTE;
        let to = item.event.change.to;
        let (outages, label, unit, colour, thresholds): (Vec<(i64, i64)>, _, _, _, Vec<Threshold>) =
            match to {
                Status::Down => (
                    vec![(item.event.change.ts_ms, NOW)],
                    Some("keine Antwort".to_owned()),
                    "ms",
                    Colour::Primary,
                    vec![Threshold {
                        value: 20.0,
                        band: Band::Warning,
                        label: "Warnung 20 ms".to_owned(),
                    }],
                ),
                Status::Up => (
                    vec![(NOW - 43 * MINUTE, NOW - MINUTE)],
                    Some("42 Min.".to_owned()),
                    "ms",
                    Colour::Primary,
                    Vec::new(),
                ),
                _ => (
                    Vec::new(),
                    None,
                    "°C",
                    Colour::Secondary,
                    vec![
                        Threshold {
                            value: 75.0,
                            band: Band::Warning,
                            label: "Warnung 75 °C".to_owned(),
                        },
                        Threshold {
                            value: 90.0,
                            band: Band::Down,
                            label: "Ausfall 90 °C".to_owned(),
                        },
                    ],
                ),
            };
        let points = (0..=360)
            .map(|k| {
                let ts = from + k * MINUTE;
                let x = k as f64 / 30.0;
                let value = if to == Status::Warning {
                    64.0 + x.sin() * 2.0
                        + if k > 240 {
                            (k - 240) as f64 * 0.12
                        } else {
                            0.0
                        }
                } else {
                    1.6 + x.sin() * 0.3 + (k as f64 * 1.7).sin() * 0.06
                };
                let down = outages.iter().any(|(a, b)| ts >= *a && ts < *b);
                (ts, (!down).then_some(value))
            })
            .collect();
        let spec = ChartSpec {
            start_ms: from,
            end_ms: NOW,
            points,
            unit: unit.to_owned(),
            thresholds,
            outages,
            outage_label: label,
            colour,
            time_zone: jiff::tz::TimeZone::get("Europe/Berlin").unwrap(),
            max_gap_ms: Some(150_000),
            decimal_comma: true,
        };
        if let Some(history) = item.details.history.as_mut() {
            history.png = Some(render_png(&spec).unwrap());
        }
        item
    }

    #[test]
    fn the_history_is_a_picture_when_there_is_one_and_bars_otherwise() {
        let ctx = context(Lang::De, None);
        let mail = compose(&ctx, &[pictured(outage(Lang::De))]);
        assert_eq!(mail.inline.len(), 2);
        assert_eq!(mail.inline[1].content_id, "chart@opennetmon");
        assert!(mail.inline[1].data.starts_with(b"\x89PNG"));
        let html = html(&mail);
        assert_self_contained(&html);
        assert!(html.contains("<img src=\"cid:chart@opennetmon\" width=\"560\""));
        assert!(
            html.contains("alt=\"Antwortzeit der letzten 6 Stunden, seit 14:31 keine Antwort\"")
        );
        assert!(
            html.contains("Antwortzeit, letzte 6 Stunden"),
            "the heading stays"
        );
        assert!(
            !html.contains("class=\"bar\""),
            "no bars next to the picture"
        );

        let mail = compose(&ctx, &[pictured(recovery(Lang::De))]);
        assert!(html_of(&mail).contains(
            "alt=\"Antwortzeit der letzten 6 Stunden mit dem Ausfall von 13:49 bis 14:31\""
        ));
        let mail = compose(&context(Lang::En, None), &[pictured(warning(Lang::En))]);
        assert!(
            html_of(&mail)
                .contains("alt=\"Temperature over the last 6 hours, above 75 °C since 14:20\"")
        );

        // Without a picture (no values, or drawing failed): the bars, and only the logo.
        let mail = compose(&ctx, &[outage(Lang::De)]);
        assert_eq!(mail.inline, [view::logo()]);
        let html = html_of(&mail);
        assert!(!html.contains("cid:chart@opennetmon") && html.contains("class=\"bar\""));
        // A summary never carries pictures.
        let mut items = burst(Lang::De, 3);
        items[0] = pictured(items[0].clone());
        assert_eq!(compose(&ctx, &items).inline.len(), 1);
    }

    #[tokio::test]
    async fn a_down_mail_carries_logo_and_chart_as_inline_images() {
        use crate::mail::testsmtp::{Behavior, TestSmtp};
        let smtp = TestSmtp::spawn(Behavior::default()).await;
        let mail = compose(
            &context(Lang::De, Some("https://onm.example.org")),
            &[pictured(outage(Lang::De))],
        );
        let config = crate::mail::MailConfig {
            enabled: true,
            host: "127.0.0.1".to_owned(),
            port: smtp.port,
            security: crate::mail::Security::None,
            username: None,
            password: None,
            from_address: "opennetmon@example.org".to_owned(),
            recipients: vec!["it@example.org".to_owned()],
            notify_warnings: true,
            language: Lang::De,
            ui_url: None,
        };
        crate::mail::send(&config, &mail).await.unwrap();
        let received = &smtp.received()[0];
        let data = &received.data;
        assert!(data.contains("Content-Type: multipart/related"), "{data}");
        assert_eq!(data.matches("Content-Type: image/png").count(), 2);
        assert!(data.contains("Content-ID: <logo@opennetmon>"));
        assert!(data.contains("Content-ID: <chart@opennetmon>"));
        let html = received.part("text/html").unwrap();
        assert!(html.contains("cid:logo@opennetmon") && html.contains("cid:chart@opennetmon"));
    }

    /// Renders sample mails to `target/mail-preview/` for a look in the browser (the logo as a
    /// `data:` URI there, only there):
    /// `cargo test --lib mail_preview -- --ignored`
    #[test]
    #[ignore = "writes preview files"]
    fn mail_preview() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/mail-preview");
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = context(Lang::De, Some("https://opennetmon.example.org"));
        let config = crate::mail::MailConfig {
            enabled: true,
            host: "mail.example.org".to_owned(),
            port: 587,
            security: crate::mail::Security::StartTls,
            username: Some("monitor".to_owned()),
            password: None,
            from_address: "opennetmon@example.org".to_owned(),
            recipients: vec!["it@example.org".to_owned(), "ops@example.org".to_owned()],
            notify_warnings: true,
            language: Lang::De,
            ui_url: Some("https://opennetmon.example.org".to_owned()),
        };
        let mails = [
            ("down", compose(&ctx, &[pictured(outage(Lang::De))])),
            ("up", compose(&ctx, &[pictured(recovery(Lang::De))])),
            ("warning", compose(&ctx, &[pictured(warning(Lang::De))])),
            ("batch", compose(&ctx, &burst(Lang::De, 14))),
            (
                "test",
                crate::mail::testmail::compose(&config, "anna", &ctx.clock),
            ),
        ];
        for (name, mail) in mails {
            let html = view::with_data_uris(mail.html.as_deref().unwrap(), &mail.inline);
            std::fs::write(dir.join(format!("{name}.html")), html).unwrap();
            std::fs::write(
                dir.join(format!("{name}.txt")),
                format!("Subject: {}\n\n{}", mail.subject, mail.text),
            )
            .unwrap();
            println!("{}", dir.join(format!("{name}.html")).display());
        }
    }
}
