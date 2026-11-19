//! Server-side line chart as a PNG, for the inline graph in alert e-mails.
//!
//! The chart is described by a [`ChartSpec`], turned into an SVG string (all text escaped), parsed by
//! `usvg`, rasterised by `resvg`/`tiny-skia` and encoded by `png`. Everything is pure Rust, so the static
//! musl build keeps linking without system libraries. Text uses the Fira Sans files in `fonts/`
//! (SIL Open Font License, embedded with `include_bytes!`); no system fonts are ever loaded.
//!
//! The picture is 560 × 196 CSS pixels, drawn at 2× (1120 × 392 pixels) so that it stays sharp when a mail
//! shows it with `width="560"`. It is opaque and always drawn on a white card with the light colours of the
//! web interface (the mail dark modes keep the card white). Look, as in the approved draft: the line with a
//! light area, gaps where there is no value, outage spans tinted red with a label to their left, dashed
//! threshold lines with labels, 4 to 5 horizontal grid lines at round values, the unit above the y axis and
//! labels at whole hours of the given time zone below.
//!
//! # Rules
//!
//! * **Input is repaired, not rejected.** Points are sorted by time; values that are `NaN`, infinite or
//!   larger than 1e300 count as "no value" (a gap in the line, like `None`); thresholds with such a value are
//!   dropped. Points outside the window are clipped (a line that crosses the window edge is cut there);
//!   spans of an outage outside the window are skipped. Only an empty or reversed window is an error.
//! * **Gaps.** `None` and invalid values break the line. With [`ChartSpec::max_gap_ms`] a larger distance
//!   between two points breaks it as well (polls that left no row at all). A single point between two gaps
//!   is drawn as a dot.
//! * **y range** (see [`y_axis`]): the range covers the visible values (with about 5 % padding) and the
//!   thresholds that are near the data (a threshold farther away than four times the spread of the data from
//!   it is left out, so that it does not flatten the line; it is then not drawn). Non-negative data starts at
//!   0, unless the visible change would use less than a quarter of the height of such a 0-based axis
//!   (temperatures 60 to 78 with limits at 85 and 95 get 40 to 100 instead of 0 to 100); constant data keeps
//!   the 0 baseline. With the unit `%` and data within 0 to 100 the axis ends at 100. The ticks are
//!   multiples of 1, 2, 2.5 or 5 times a power of ten, at most four intervals (4 to 5 grid lines).
//! * **Labels** use Fira Sans 11 px. Text is measured with the same font engine, so the left margin grows
//!   with wide tick labels and the label of an outage goes to the left of the span (right-aligned, ending
//!   with an arrow); only if there is no room it goes to the right or inside. Only the last outage of the
//!   window gets the label. Texts are cut to 48 characters (the unit to 16).
//! * **Size.** Pass at most a few thousand points (6 hours at 10 s are 2160); the path grows with them.
//!
//! The call is CPU bound (a few milliseconds in release builds); async callers should use
//! `tokio::task::spawn_blocking`.

use std::sync::{Arc, OnceLock};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use resvg::{tiny_skia, usvg};

/// Width of the picture in CSS pixels (the mail shows it with `width="560"`).
pub const WIDTH: u32 = 560;
/// Height of the picture in CSS pixels.
pub const HEIGHT: u32 = 196;
/// Pixels per CSS pixel; the PNG is `WIDTH * SCALE` × `HEIGHT * SCALE`.
pub const SCALE: u32 = 2;

const FONT_FAMILY: &str = "Fira Sans";
const FONT_SIZE: f64 = 11.0;
static FIRA_SANS_REGULAR: &[u8] = include_bytes!("../fonts/FiraSans-Regular.ttf");
static FIRA_SANS_MEDIUM: &[u8] = include_bytes!("../fonts/FiraSans-Medium.ttf");

// Light-theme tokens of static/app.css.
const WHITE: &str = "#ffffff";
const GRID: &str = "#e6e9ed";
const AXIS: &str = "#c3cad2"; // --border-strong
const TEXT_3: &str = "#636c78";
const SERIES_1: &str = "#1c5ea8"; // --s1
const SERIES_2: &str = "#b45309"; // --s2
const WARN: &str = "#7a5600";
const ERR: &str = "#a1132c";
const ERR_BG: &str = "#fbe8eb";

// Plot area (CSS pixels): the margins of the draft; the left one grows with wide tick labels.
const MARGIN_TOP: f64 = 24.0;
const MARGIN_BOTTOM: f64 = 26.0;
const MARGIN_RIGHT: f64 = 14.0;
const MARGIN_LEFT_MIN: f64 = 44.0;
const MARGIN_LEFT_MAX: f64 = 120.0;

const MAX_LABEL_CHARS: usize = 48;
const MAX_UNIT_CHARS: usize = 16;
/// Values with a larger magnitude are treated as "no value" (keeps all arithmetic far from overflow).
const MAX_ABS_VALUE: f64 = 1e300;

/// Colour of a threshold line and its label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    /// Warning limit (`--warn`).
    Warning,
    /// Error limit (`--err`).
    Down,
}

/// A dashed horizontal line at `value` with a short label (for example `Warnung > 80 °C`).
#[derive(Debug, Clone, PartialEq)]
pub struct Threshold {
    pub value: f64,
    pub band: Band,
    /// May be empty (line only).
    pub label: String,
}

/// Colour of the data line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    /// `--s1`, blue.
    Primary,
    /// `--s2`, orange.
    Secondary,
}

/// Everything the chart shows.
#[derive(Debug, Clone)]
pub struct ChartSpec {
    /// Start of the window in milliseconds since the Unix epoch (UTC); must be before `end_ms`.
    pub start_ms: i64,
    /// End of the window; the whole picture spans `start_ms..end_ms` (6 hours in practice).
    pub end_ms: i64,
    /// Time and value; `None` is "no answer" and breaks the line. Any order, any range.
    pub points: Vec<(i64, Option<f64>)>,
    /// Unit shown once above the y axis, for example `ms`, `°C` or `%`; may be empty.
    pub unit: String,
    /// Dashed lines; those outside the visible value range are not drawn.
    pub thresholds: Vec<Threshold>,
    /// Spans to tint as "down", `(from_ms, to_ms)`.
    pub outages: Vec<(i64, i64)>,
    /// Short text next to the last outage, for example `keine Antwort` or `12 Min.`.
    pub outage_label: Option<String>,
    pub colour: Colour,
    /// Time zone of the x axis labels.
    pub time_zone: TimeZone,
    /// A larger distance between two neighbouring points breaks the line (`None`: never).
    pub max_gap_ms: Option<i64>,
    /// Decimal comma in the tick labels (`2,5` instead of `2.5`), for German.
    pub decimal_comma: bool,
}

/// Why a chart could not be produced.
#[derive(Debug, thiserror::Error)]
pub enum ChartError {
    #[error("the window of the chart must end after it starts")]
    InvalidWindow,
    #[error("SVG error: {0}")]
    Svg(String),
    #[error("could not allocate the pixel buffer")]
    Pixmap,
    #[error("PNG encoding failed: {0}")]
    Png(String),
}

/// Draws the chart and returns the PNG (opaque RGB, 8 bit, `WIDTH * SCALE` × `HEIGHT * SCALE` pixels).
pub fn render_png(spec: &ChartSpec) -> Result<Vec<u8>, ChartError> {
    let svg = render_svg(spec)?;
    let tree = parse(&svg)?;
    let (w, h) = (WIDTH * SCALE, HEIGHT * SCALE);
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or(ChartError::Pixmap)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(SCALE as f32, SCALE as f32),
        &mut pixmap.as_mut(),
    );
    encode_png(&pixmap)
}

/// The SVG of the chart (560 × 196 user units); [`render_png`] rasterises it.
pub fn render_svg(spec: &ChartSpec) -> Result<String, ChartError> {
    if spec.end_ms <= spec.start_ms {
        return Err(ChartError::InvalidWindow);
    }
    let start = spec.start_ms as f64;
    let span = spec.end_ms as f64 - start;

    let shapes = Shapes::from_spec(spec);
    let thresholds: Vec<&Threshold> = spec.thresholds.iter().filter(|t| usable(t.value)).collect();
    let threshold_values: Vec<f64> = thresholds.iter().map(|t| t.value).collect();
    let axis = y_axis(&shapes.values(), &threshold_values, spec.unit.trim() == "%");
    let shown: Vec<&Threshold> = thresholds
        .into_iter()
        .filter(|t| t.value >= axis.min && t.value <= axis.max)
        .collect();
    let format = TickFormat::for_axis(&axis);
    let y_labels: Vec<String> = axis
        .ticks
        .iter()
        .map(|&v| format.apply(v, spec.decimal_comma))
        .collect();
    let x_ticks = x_ticks(spec.start_ms, spec.end_ms, &spec.time_zone);
    let unit = clean(&spec.unit, MAX_UNIT_CHARS);
    let outage_text = spec
        .outage_label
        .as_deref()
        .map(|l| clean(l, MAX_LABEL_CHARS))
        .filter(|l| !l.is_empty());

    // Measure the texts that influence the layout in one go (regular: ticks; medium: threshold and outage labels).
    let mut texts: Vec<(String, bool)> = Vec::new();
    let y_at = texts.len();
    texts.extend(y_labels.iter().map(|l| (l.clone(), false)));
    let x_at = texts.len();
    texts.extend(x_ticks.iter().map(|(_, l)| (l.clone(), false)));
    let threshold_at = texts.len();
    texts.extend(
        shown
            .iter()
            .map(|t| (clean(&t.label, MAX_LABEL_CHARS), true)),
    );
    let outage_at = texts.len();
    if let Some(label) = &outage_text {
        texts.push((format!("{label} \u{2192}"), true));
        texts.push((format!("\u{2190} {label}"), true));
        texts.push((label.clone(), true));
    }
    let widths = measure(&texts)?;

    let widest_tick = widths[y_at..x_at].iter().copied().fold(0.0, f64::max);
    let left = (widest_tick + 12.0)
        .ceil()
        .clamp(MARGIN_LEFT_MIN, MARGIN_LEFT_MAX);
    let plot = Plot {
        left,
        right: f64::from(WIDTH) - MARGIN_RIGHT,
        top: MARGIN_TOP,
        bottom: f64::from(HEIGHT) - MARGIN_BOTTOM,
    };
    let x_of =
        |t: f64| (plot.left + (t - start) / span * plot.width()).clamp(plot.left, plot.right);
    let y_of = |v: f64| {
        (plot.bottom - (v - axis.min) / (axis.max - axis.min) * plot.height())
            .clamp(plot.top, plot.bottom)
    };
    // Lines of the grid sit on whole physical pixels (multiples of 0.5 CSS pixels at 2×).
    let snap = |y: f64| (y * 2.0).round() / 2.0;
    let colour = match spec.colour {
        Colour::Primary => SERIES_1,
        Colour::Secondary => SERIES_2,
    };

    let mut svg = String::with_capacity(8 * 1024);
    svg.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}" viewBox="0 0 {WIDTH} {HEIGHT}" font-family="{FONT_FAMILY}" font-size="{}">"#,
        num(FONT_SIZE)
    ));
    svg.push_str(&format!(
        r#"<rect width="{WIDTH}" height="{HEIGHT}" fill="{WHITE}"/>"#
    ));

    // Outages, tinted behind everything else.
    let mut tints: Vec<(f64, f64)> = Vec::new();
    let mut last_outage: Option<(f64, f64)> = None;
    for &(from, to) in &spec.outages {
        if to < from || to < spec.start_ms || from > spec.end_ms {
            continue;
        }
        let x1 = x_of(from.max(spec.start_ms) as f64).min(plot.right - 1.5);
        let x2 = x_of(to.min(spec.end_ms) as f64).max(x1 + 1.5);
        svg.push_str(&format!(
            r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{ERR_BG}"/>"#,
            num(x1),
            num(plot.top),
            num(x2 - x1),
            num(plot.height())
        ));
        tints.push((x1, x2));
        if last_outage.is_none_or(|(prev, _)| x1 >= prev) {
            last_outage = Some((x1, x2));
        }
    }

    // Grid and the baseline.
    let mut grid = String::new();
    for &tick in &axis.ticks {
        let y = snap(y_of(tick));
        grid.push_str(&format!(
            "M{},{}H{}",
            num(plot.left),
            num(y),
            num(plot.right)
        ));
    }
    svg.push_str(&format!(
        r#"<path d="{grid}" fill="none" stroke="{GRID}" stroke-width="1"/>"#
    ));
    let base = snap(plot.bottom);
    svg.push_str(&format!(
        r#"<path d="M{},{}H{}" fill="none" stroke="{AXIS}" stroke-width="1"/>"#,
        num(plot.left),
        num(base),
        num(plot.right)
    ));

    // Threshold lines.
    for t in &shown {
        svg.push_str(&format!(
            r#"<path d="M{},{}H{}" fill="none" stroke="{}" stroke-width="1.5" stroke-dasharray="5 4"/>"#,
            num(plot.left),
            num(snap(y_of(t.value))),
            num(plot.right),
            band_colour(t.band)
        ));
    }

    // Area, line and single points.
    let (mut line, mut area) = (String::new(), String::new());
    for poly in &shapes.lines {
        let pts: Vec<(f64, f64)> = poly.iter().map(|&(t, v)| (x_of(t), y_of(v))).collect();
        let (Some(first), Some(last)) = (pts.first(), pts.last()) else {
            continue;
        };
        for (i, (x, y)) in pts.iter().enumerate() {
            let cmd = if i == 0 { 'M' } else { 'L' };
            line.push_str(&format!("{cmd}{},{}", num(*x), num(*y)));
        }
        area.push_str(&format!(
            "M{},{}{}L{},{}Z",
            num(first.0),
            num(plot.bottom),
            pts.iter()
                .map(|(x, y)| format!("L{},{}", num(*x), num(*y)))
                .collect::<String>(),
            num(last.0),
            num(plot.bottom)
        ));
    }
    if !area.is_empty() {
        svg.push_str(&format!(
            r#"<path d="{area}" fill="{colour}" fill-opacity=".1"/>"#
        ));
        svg.push_str(&format!(
            r#"<path d="{line}" fill="none" stroke="{colour}" stroke-width="2" stroke-linejoin="round" stroke-linecap="round"/>"#
        ));
    }
    for &(t, v) in &shapes.dots {
        svg.push_str(&format!(
            r#"<circle cx="{}" cy="{}" r="2.5" fill="{colour}"/>"#,
            num(x_of(t)),
            num(y_of(v))
        ));
    }

    // Texts. y tick labels, the unit above the axis.
    for (i, &tick) in axis.ticks.iter().enumerate() {
        svg.push_str(&text(
            plot.left - 6.0,
            snap(y_of(tick)) + 4.0,
            "end",
            TEXT_3,
            false,
            &y_labels[i],
        ));
    }
    if !unit.is_empty() {
        svg.push_str(&text(
            plot.left,
            plot.top - 10.0,
            "start",
            TEXT_3,
            false,
            &unit,
        ));
    }
    // x labels: centred below their hour, the first and last aligned to the edges of the plot.
    let label_y = f64::from(HEIGHT) - 8.0;
    for (i, (t, label)) in x_ticks.iter().enumerate() {
        let (x, w) = (x_of(*t as f64), widths[x_at + i]);
        let (anchor, x) = if (x - plot.left).abs() < 0.5 {
            ("start", x)
        } else if (x - plot.right).abs() < 0.5 {
            ("end", x)
        } else {
            let half = w / 2.0;
            ("middle", x.clamp(2.0 + half, f64::from(WIDTH) - 2.0 - half))
        };
        svg.push_str(&text(x, label_y, anchor, TEXT_3, false, label));
    }

    // Labels with a white halo (not over a tinted span, where it would show), placed so that they do not collide.
    let mut taken: Vec<Rect> = Vec::new();
    let over_tint = |r: &Rect| tints.iter().any(|&(a, b)| a < r.x1 && r.x0 < b);
    if let (Some(label), Some((x1, x2))) = (&outage_text, last_outage) {
        let baseline = plot.top + 14.0;
        let (w_left, w_right, w_plain) = (
            widths[outage_at],
            widths[outage_at + 1],
            widths[outage_at + 2],
        );
        let (text_, x, anchor, w) = if x1 - 6.0 - w_left >= plot.left + 2.0 {
            (format!("{label} \u{2192}"), x1 - 6.0, "end", w_left)
        } else if x2 + 6.0 + w_right <= plot.right - 2.0 {
            (format!("\u{2190} {label}"), x2 + 6.0, "start", w_right)
        } else {
            (label.clone(), plot.left + 6.0, "start", w_plain)
        };
        let x0 = if anchor == "end" { x - w } else { x };
        let area = Rect {
            x0,
            y0: baseline - 9.0,
            x1: x0 + w,
            y1: baseline + 3.0,
        };
        svg.push_str(&label_text(
            x,
            baseline,
            anchor,
            ERR,
            &text_,
            !over_tint(&area),
        ));
        taken.push(area);
    }
    let mut ordered: Vec<(usize, &Threshold)> = shown.iter().copied().enumerate().collect();
    ordered.sort_by(|a, b| b.1.value.total_cmp(&a.1.value));
    for (i, t) in ordered {
        let label = clean(&t.label, MAX_LABEL_CHARS);
        if label.is_empty() {
            continue;
        }
        let w = widths[threshold_at + i];
        let (x1, y) = (plot.right - 4.0, snap(y_of(t.value)));
        let fits = |baseline: f64| Rect {
            x0: x1 - w,
            y0: baseline - 9.0,
            x1,
            y1: baseline + 3.0,
        };
        let above = fits(y - 4.0);
        let below = fits(y + 12.0);
        let baseline = if above.y0 >= 2.0 && !taken.iter().any(|r| r.hits(&above)) {
            y - 4.0
        } else if below.y1 <= plot.bottom && !taken.iter().any(|r| r.hits(&below)) {
            y + 12.0
        } else {
            y - 4.0
        };
        let area = fits(baseline);
        let colour = band_colour(t.band);
        svg.push_str(&label_text(
            x1,
            baseline,
            "end",
            colour,
            &label,
            !over_tint(&area),
        ));
        taken.push(area);
    }

    svg.push_str("</svg>");
    Ok(svg)
}

// ---------------------------------------------------------------------------------------------------------
// Rendering

fn font_database() -> Arc<usvg::fontdb::Database> {
    static DATABASE: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DATABASE
        .get_or_init(|| {
            use usvg::fontdb::{Database, Source};
            let mut db = Database::new();
            db.load_font_source(Source::Binary(Arc::new(FIRA_SANS_REGULAR)));
            db.load_font_source(Source::Binary(Arc::new(FIRA_SANS_MEDIUM)));
            // Whatever a font-family list resolves to, it must end at Fira Sans.
            db.set_serif_family(FONT_FAMILY);
            db.set_sans_serif_family(FONT_FAMILY);
            db.set_cursive_family(FONT_FAMILY);
            db.set_fantasy_family(FONT_FAMILY);
            db.set_monospace_family(FONT_FAMILY);
            Arc::new(db)
        })
        .clone()
}

fn parse(svg: &str) -> Result<usvg::Tree, ChartError> {
    let options = usvg::Options {
        font_family: FONT_FAMILY.to_owned(),
        font_size: FONT_SIZE as f32,
        fontdb: font_database(),
        ..usvg::Options::default()
    };
    usvg::Tree::from_str(svg, &options).map_err(|e| ChartError::Svg(e.to_string()))
}

/// Advance width in CSS pixels of each text at 11 px (`true`: Medium weight, `false`: Regular).
fn measure(texts: &[(String, bool)]) -> Result<Vec<f64>, ChartError> {
    let mut widths = vec![0.0; texts.len()];
    if texts.is_empty() {
        return Ok(widths);
    }
    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" font-family="{FONT_FAMILY}" font-size="{}">"#,
        num(FONT_SIZE)
    );
    for (i, (text, medium)) in texts.iter().enumerate() {
        if text.is_empty() {
            continue;
        }
        svg.push_str(&format!(
            r#"<text id="m{i}" x="0" y="10" font-weight="{}">{}</text>"#,
            if *medium { 500 } else { 400 },
            escape(text)
        ));
    }
    svg.push_str("</svg>");
    let tree = parse(&svg)?;
    collect_widths(tree.root(), &mut widths);
    Ok(widths)
}

fn collect_widths(group: &usvg::Group, widths: &mut [f64]) {
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => collect_widths(g, widths),
            usvg::Node::Text(t) => {
                let index = t
                    .id()
                    .strip_prefix('m')
                    .and_then(|n| n.parse::<usize>().ok());
                if let Some(slot) = index.and_then(|i| widths.get_mut(i)) {
                    *slot = f64::from(t.bounding_box().width());
                }
            }
            _ => {}
        }
    }
}

fn encode_png(pixmap: &tiny_skia::Pixmap) -> Result<Vec<u8>, ChartError> {
    // The picture is opaque (white background), so premultiplied and plain colours are the same; drop alpha.
    let mut rgb = Vec::with_capacity(pixmap.pixels().len() * 3);
    for p in pixmap.pixels() {
        rgb.extend_from_slice(&[p.red(), p.green(), p.blue()]);
    }
    let mut out = Vec::with_capacity(32 * 1024);
    let fail = |e: png::EncodingError| ChartError::Png(e.to_string());
    let mut encoder = png::Encoder::new(&mut out, pixmap.width(), pixmap.height());
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    let mut writer = encoder.write_header().map_err(fail)?;
    writer.write_image_data(&rgb).map_err(fail)?;
    writer.finish().map_err(fail)?;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------
// SVG helpers

struct Plot {
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
}

impl Plot {
    fn width(&self) -> f64 {
        self.right - self.left
    }
    fn height(&self) -> f64 {
        self.bottom - self.top
    }
}

struct Rect {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Rect {
    fn hits(&self, other: &Rect) -> bool {
        self.x0 < other.x1 && other.x0 < self.x1 && self.y0 < other.y1 && other.y0 < self.y1
    }
}

fn band_colour(band: Band) -> &'static str {
    match band {
        Band::Warning => WARN,
        Band::Down => ERR,
    }
}

/// A coordinate with at most two decimals and no trailing zeros.
fn num(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" {
        "0".to_owned()
    } else {
        s.to_owned()
    }
}

fn text(x: f64, y: f64, anchor: &str, fill: &str, medium: bool, content: &str) -> String {
    format!(
        r#"<text x="{}" y="{}" text-anchor="{anchor}" fill="{fill}"{}>{}</text>"#,
        num(x),
        num(y),
        if medium { r#" font-weight="500""# } else { "" },
        escape(content)
    )
}

/// Medium-weight label, with a white outline underneath (`halo`) so that lines behind it do not hurt legibility.
fn label_text(x: f64, y: f64, anchor: &str, fill: &str, content: &str, halo: bool) -> String {
    let outline = if halo {
        format!(
            r#" stroke="{WHITE}" stroke-width="3" stroke-linejoin="round" paint-order="stroke""#
        )
    } else {
        String::new()
    };
    format!(
        r#"<text x="{}" y="{}" text-anchor="{anchor}" fill="{fill}" font-weight="500"{outline}>{}</text>"#,
        num(x),
        num(y),
        escape(content)
    )
}

/// Escapes text for XML; characters that XML 1.0 does not allow become spaces.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(' '),
            c if (c as u32) < 0x20 || c == '\u{7f}' || c == '\u{fffe}' || c == '\u{ffff}' => {
                out.push(' ')
            }
            c => out.push(c),
        }
    }
    out
}

/// Trimmed text of at most `max` characters (an ellipsis marks a cut), control characters as spaces.
fn clean(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        let mut cut: String = flat.chars().take(max.saturating_sub(1)).collect();
        cut.push('\u{2026}');
        cut
    }
}

// ---------------------------------------------------------------------------------------------------------
// Data

fn usable(v: f64) -> bool {
    v.is_finite() && v.abs() < MAX_ABS_VALUE
}

/// What is drawn: polylines (at least two vertices, clipped to the window) and isolated points.
struct Shapes {
    lines: Vec<Vec<(f64, f64)>>,
    dots: Vec<(f64, f64)>,
}

impl Shapes {
    fn from_spec(spec: &ChartSpec) -> Shapes {
        let (lo, hi) = (spec.start_ms as f64, spec.end_ms as f64);
        let mut points = spec.points.clone();
        points.sort_by_key(|p| p.0);

        // Runs of consecutive valid points.
        let mut runs: Vec<Vec<(f64, f64)>> = Vec::new();
        let mut current: Vec<(f64, f64)> = Vec::new();
        let mut previous: Option<i64> = None;
        for (ts, value) in points {
            match value.filter(|v| usable(*v)) {
                Some(v) => {
                    let too_far = matches!((spec.max_gap_ms, previous), (Some(gap), Some(prev)) if ts.saturating_sub(prev) > gap);
                    if too_far && !current.is_empty() {
                        runs.push(std::mem::take(&mut current));
                    }
                    current.push((ts as f64, v));
                    previous = Some(ts);
                }
                None => {
                    if !current.is_empty() {
                        runs.push(std::mem::take(&mut current));
                    }
                }
            }
        }
        if !current.is_empty() {
            runs.push(current);
        }

        let mut shapes = Shapes {
            lines: Vec::new(),
            dots: Vec::new(),
        };
        for run in runs {
            if let [only] = run.as_slice() {
                if only.0 >= lo && only.0 <= hi {
                    shapes.dots.push(*only);
                }
                continue;
            }
            let mut poly: Vec<(f64, f64)> = Vec::new();
            for pair in run.windows(2) {
                if let Some((a, b)) = clip_segment(pair[0], pair[1], lo, hi) {
                    if poly.last() != Some(&a) {
                        poly.push(a);
                    }
                    poly.push(b);
                }
            }
            match poly.len() {
                0 => {}
                1 => shapes.dots.push(poly[0]),
                _ => shapes.lines.push(poly),
            }
        }
        shapes
    }

    fn values(&self) -> Vec<f64> {
        self.lines
            .iter()
            .flatten()
            .chain(self.dots.iter())
            .map(|&(_, v)| v)
            .collect()
    }
}

/// The part of the segment `a`–`b` (`a` not after `b`) inside `lo..=hi`, cut by linear interpolation.
fn clip_segment(
    a: (f64, f64),
    b: (f64, f64),
    lo: f64,
    hi: f64,
) -> Option<((f64, f64), (f64, f64))> {
    if a.0 > hi || b.0 < lo {
        return None;
    }
    let at = |t: f64| {
        if b.0 == a.0 {
            a.1
        } else {
            a.1 + (b.1 - a.1) * (t - a.0) / (b.0 - a.0)
        }
    };
    let start = if a.0 < lo { (lo, at(lo)) } else { a };
    let end = if b.0 > hi { (hi, at(hi)) } else { b };
    Some((start, end))
}

// ---------------------------------------------------------------------------------------------------------
// Axes

/// The value axis: range and tick values (`min`/`max` are ticks themselves).
#[derive(Debug, Clone, PartialEq)]
pub struct YAxis {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub ticks: Vec<f64>,
}

/// Range and ticks for the visible `values` and the `thresholds`; the rules are in the module
/// documentation. `percent`: the unit is `%` (the axis then ends at 100 if all data are within 0 to 100).
pub fn y_axis(values: &[f64], thresholds: &[f64], percent: bool) -> YAxis {
    let data = min_max(values);
    // Thresholds far from the data would flatten it: keep those within four spreads of the data.
    let kept: Vec<f64> = match data {
        None => thresholds.to_vec(),
        Some((lo, hi)) => {
            let scale = lo.abs().max(hi.abs());
            if scale == 0.0 {
                thresholds.to_vec()
            } else {
                let reach = 4.0 * (hi - lo).max(0.25 * scale);
                thresholds
                    .iter()
                    .copied()
                    .filter(|&t| (lo - t).max(t - hi).max(0.0) <= reach)
                    .collect()
            }
        }
    };
    let all = [values, kept.as_slice()].concat();
    let Some((lo, hi)) = min_max(&all) else {
        return build_axis(0.0, 1.0);
    };
    let (data_lo, data_hi) = data.unwrap_or((lo, hi));
    let non_negative = lo >= 0.0;
    // 0-based unless the data would use less than a quarter of the height of the 0-based axis.
    let zero_based = non_negative
        && (data.is_none() || hi == 0.0 || data_hi == data_lo || (data_hi - data_lo) / hi >= 0.25);

    let (mut min, mut max) = if zero_based {
        (0.0, if hi > 0.0 { hi * 1.05 } else { 1.0 })
    } else {
        let pad = if hi > lo {
            0.05 * (hi - lo)
        } else if lo != 0.0 {
            0.1 * lo.abs()
        } else {
            1.0
        };
        (
            if non_negative {
                (lo - pad).max(0.0)
            } else {
                lo - pad
            },
            hi + pad,
        )
    };
    if percent && non_negative && hi <= 100.0 {
        max = max.min(100.0);
    }
    // A range too small to divide (denormal numbers) would give a step of 0.
    if !(min.is_finite() && max.is_finite()) || max - min < 1e-280 {
        min = 0.0;
        max = 1.0;
    }
    build_axis(min, max)
}

fn min_max(values: &[f64]) -> Option<(f64, f64)> {
    values.iter().copied().fold(None, |acc, v| match acc {
        None => Some((v, v)),
        Some((lo, hi)) => Some((lo.min(v), hi.max(v))),
    })
}

/// Round ticks around `lo..hi` (the axis is widened to whole steps).
fn build_axis(lo: f64, hi: f64) -> YAxis {
    let step = nice_step(lo, hi);
    if !(step.is_finite() && step > 0.0) {
        return build_axis(0.0, 1.0);
    }
    let first = (lo / step + 1e-9).floor() as i64;
    let last = (hi / step - 1e-9).ceil() as i64;
    let ticks: Vec<f64> = (first..=last).map(|k| k as f64 * step).collect();
    match (ticks.first(), ticks.last()) {
        (Some(&min), Some(&max)) if max > min => YAxis {
            min,
            max,
            step,
            ticks,
        },
        _ => YAxis {
            min: 0.0,
            max: 1.0,
            step: 0.25,
            ticks: vec![0.0, 0.25, 0.5, 0.75, 1.0],
        },
    }
}

/// Smallest of 1, 2, 2.5 and 5 times a power of ten that needs at most four intervals for `lo..hi`.
fn nice_step(lo: f64, hi: f64) -> f64 {
    let intervals = |step: f64| (hi / step - 1e-9).ceil() - (lo / step + 1e-9).floor();
    let mut magnitude = 10f64.powi(((hi - lo) / 4.0).log10().floor() as i32);
    if !magnitude.is_normal() {
        return 1.0;
    }
    for _ in 0..12 {
        for factor in [1.0, 2.0, 2.5, 5.0] {
            let step = factor * magnitude;
            if intervals(step) <= 4.0 {
                return step;
            }
        }
        magnitude *= 10.0;
    }
    magnitude
}

/// How tick values are written: digits after the decimal point, a divisor with suffix for huge values.
struct TickFormat {
    decimals: usize,
    divisor: f64,
    suffix: &'static str,
    scientific: bool,
}

impl TickFormat {
    fn for_axis(axis: &YAxis) -> TickFormat {
        let largest = axis.min.abs().max(axis.max.abs());
        if axis.step < 1e-5 || largest >= 1e15 {
            return TickFormat {
                decimals: 1,
                divisor: 1.0,
                suffix: "",
                scientific: true,
            };
        }
        let (divisor, suffix) = if largest >= 1e12 {
            (1e12, "T")
        } else if largest >= 1e9 {
            (1e9, "G")
        } else if largest >= 1e6 {
            (1e6, "M")
        } else {
            (1.0, "")
        };
        TickFormat {
            decimals: decimals_for(axis.step / divisor),
            divisor,
            suffix,
            scientific: false,
        }
    }

    fn apply(&self, value: f64, comma: bool) -> String {
        if value == 0.0 {
            return "0".to_owned();
        }
        let mut s = if self.scientific {
            format!("{:.*e}", self.decimals, value)
        } else {
            format!("{:.*}{}", self.decimals, value / self.divisor, self.suffix)
        };
        if s.starts_with('-') && !s.chars().any(|c| c.is_ascii_digit() && c != '0') {
            s.remove(0); // "-0" and "-0.0" are 0
        }
        if comma { s.replace('.', ",") } else { s }
    }
}

/// Digits after the point needed to show multiples of `step` exactly (at most 6).
fn decimals_for(step: f64) -> usize {
    (0..6)
        .find(|&d| {
            let scaled = step * 10f64.powi(d);
            (scaled - scaled.round()).abs() <= 1e-6 * scaled.abs().max(1.0)
        })
        .map_or(6, |d| d as usize)
}

/// Labels at whole hours (or finer or coarser for other windows) of `zone`, at most about eight:
/// `(timestamp_ms, "HH:MM")`. Windows longer than a week get none.
pub fn x_ticks(start_ms: i64, end_ms: i64, zone: &TimeZone) -> Vec<(i64, String)> {
    const STEPS_MIN: [i64; 12] = [1, 2, 5, 10, 15, 30, 60, 120, 180, 360, 720, 1440];
    let Ok(start) = Timestamp::from_millisecond(start_ms) else {
        return Vec::new();
    };
    let offset_ms = i64::from(zone.to_offset(start).seconds()) * 1000;
    let tick_times = |step_ms: i64| -> Vec<i64> {
        // Local time = UTC + offset; ticks are multiples of the step in local time.
        let first = (start_ms + offset_ms).div_euclid(step_ms) * step_ms - offset_ms;
        let first = if first < start_ms {
            first + step_ms
        } else {
            first
        };
        std::iter::successors(Some(first), |t| t.checked_add(step_ms))
            .take_while(|t| *t <= end_ms)
            .take(64)
            .collect()
    };
    let times = STEPS_MIN
        .iter()
        .map(|m| tick_times(m * 60_000))
        .find(|t| t.len() <= 8)
        .unwrap_or_default();
    times
        .into_iter()
        .filter_map(|t| {
            let local = zone.to_datetime(Timestamp::from_millisecond(t).ok()?);
            Some((t, format!("{:02}:{:02}", local.hour(), local.minute())))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600_000;

    type Points = Vec<(i64, Option<f64>)>;

    fn berlin() -> TimeZone {
        TimeZone::get("Europe/Berlin").unwrap()
    }

    /// 2026-11-19 09:00 UTC, which is 10:00 in Berlin.
    fn window() -> (i64, i64) {
        let start = "2026-11-19T09:00:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .as_millisecond();
        (start, start + 6 * HOUR)
    }

    fn spec(points: Vec<(i64, Option<f64>)>) -> ChartSpec {
        let (start_ms, end_ms) = window();
        ChartSpec {
            start_ms,
            end_ms,
            points,
            unit: "ms".into(),
            thresholds: vec![],
            outages: vec![],
            outage_label: None,
            colour: Colour::Primary,
            time_zone: berlin(),
            max_gap_ms: None,
            decimal_comma: true,
        }
    }

    /// 72 points every 5 minutes with a bump, the last `gap` of them without answer.
    fn ping(gap: usize) -> ChartSpec {
        let (start, end) = window();
        let n = 72;
        let points = (0..n)
            .map(|i| {
                let t = start + (end - start) * i as i64 / n as i64;
                let v = 1.6 + 0.5 * (i as f64 / 9.0).sin() + (i % 7) as f64 * 0.1;
                (t, (i < n - gap).then_some(v))
            })
            .collect();
        spec(points)
    }

    fn png_size(png: &[u8]) -> (u32, u32) {
        let be = |i: usize| u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]);
        (be(16), be(20))
    }

    fn pixels(png: &[u8]) -> Vec<[u8; 3]> {
        let mut reader = png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgb);
        buf[..info.buffer_size()]
            .chunks(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect()
    }

    fn has_colour(png: &[u8], hex: &str) -> bool {
        let rgb = u32::from_str_radix(&hex[1..], 16).unwrap();
        let want = [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8];
        pixels(png).contains(&want)
    }

    #[test]
    fn png_has_signature_and_double_size() {
        let png = render_png(&ping(0)).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(png_size(&png), (1120, 392));
        assert!(png.len() < 40_000, "{} bytes", png.len());
        assert_eq!(pixels(&png)[0], [255, 255, 255]);
    }

    #[test]
    fn output_is_deterministic() {
        let mut s = ping(10);
        s.outages = vec![(window().1 - HOUR, window().1)];
        s.outage_label = Some("keine Antwort".into());
        s.thresholds = vec![Threshold {
            value: 3.0,
            band: Band::Warning,
            label: "Warnung > 3 ms".into(),
        }];
        assert_eq!(render_png(&s).unwrap(), render_png(&s).unwrap());
    }

    #[test]
    fn draws_line_tint_and_thresholds_in_the_theme_colours() {
        let mut s = ping(10);
        s.outages = vec![(window().1 - HOUR, window().1)];
        s.thresholds = vec![
            Threshold {
                value: 2.8,
                band: Band::Warning,
                label: "w".into(),
            },
            Threshold {
                value: 3.4,
                band: Band::Down,
                label: "d".into(),
            },
        ];
        let png = render_png(&s).unwrap();
        assert!(has_colour(&png, "#1c5ea8"), "line");
        assert!(has_colour(&png, "#fbe8eb"), "tint");
        assert!(has_colour(&png, "#7a5600"), "warning line");
        assert!(has_colour(&png, "#a1132c"), "error line");
        assert!(has_colour(&png, "#e6e9ed"), "grid");
        let plain = render_png(&ping(0)).unwrap();
        assert!(!has_colour(&plain, "#fbe8eb"));
        assert!(!has_colour(&plain, "#a1132c"));
        let mut orange = ping(0);
        orange.colour = Colour::Secondary;
        assert!(has_colour(&render_png(&orange).unwrap(), "#b45309"));
    }

    #[test]
    fn text_is_rendered_with_the_embedded_font() {
        let widths = measure(&[
            ("0000".into(), false),
            ("0000".into(), true),
            ("00000000".into(), false),
            (String::new(), false),
        ])
        .unwrap();
        assert!((20.0..36.0).contains(&widths[0]), "{widths:?}");
        assert!(widths[1] > widths[0], "medium is wider: {widths:?}");
        assert!((widths[2] / widths[0] - 2.0).abs() < 0.05, "{widths:?}");
        assert_eq!(widths[3], 0.0);
        // The tick labels leave dark pixels left of the plot.
        let png = render_png(&ping(0)).unwrap();
        let w = (WIDTH * SCALE) as usize;
        let px = pixels(&png);
        let dark = (0..(HEIGHT * SCALE) as usize)
            .flat_map(|y| (0..88).map(move |x| (x, y)))
            .filter(|&(x, y)| px[y * w + x][0] < 200)
            .count();
        assert!(dark > 20, "{dark}");
    }

    #[test]
    fn edge_cases_render() {
        let (start, end) = window();
        let mid = start + 3 * HOUR;
        let cases: Vec<(&str, Points)> = vec![
            ("no points", vec![]),
            ("all none", vec![(start, None), (mid, None), (end, None)]),
            ("one point", vec![(mid, Some(5.0))]),
            ("one point at the edge", vec![(start, Some(5.0))]),
            (
                "constant",
                (0..10)
                    .map(|i| (start + i * HOUR / 2, Some(42.0)))
                    .collect(),
            ),
            (
                "constant zero",
                (0..10).map(|i| (start + i * HOUR / 2, Some(0.0))).collect(),
            ),
            (
                "constant negative",
                (0..10)
                    .map(|i| (start + i * HOUR / 2, Some(-7.5)))
                    .collect(),
            ),
            (
                "negative",
                (0..10)
                    .map(|i| (start + i * HOUR / 2, Some(i as f64 - 5.0)))
                    .collect(),
            ),
            (
                "huge",
                (0..10)
                    .map(|i| (start + i * HOUR / 2, Some(1e15 * (i + 1) as f64)))
                    .collect(),
            ),
            (
                "tiny",
                (0..10)
                    .map(|i| (start + i * HOUR / 2, Some(1e-12 * (i + 1) as f64)))
                    .collect(),
            ),
            (
                "max float",
                vec![
                    (start, Some(f64::MAX)),
                    (mid, Some(f64::MIN)),
                    (end, Some(1.0)),
                ],
            ),
            (
                "nan and infinity",
                vec![
                    (start, Some(f64::NAN)),
                    (mid, Some(f64::INFINITY)),
                    (end, Some(f64::NEG_INFINITY)),
                ],
            ),
            (
                "nan between values",
                vec![
                    (start, Some(1.0)),
                    (start + HOUR, Some(f64::NAN)),
                    (start + 2 * HOUR, Some(2.0)),
                ],
            ),
            (
                "unsorted",
                vec![
                    (end, Some(3.0)),
                    (start, Some(1.0)),
                    (mid, Some(2.0)),
                    (start + HOUR, Some(1.5)),
                ],
            ),
            (
                "same timestamp",
                vec![(mid, Some(1.0)), (mid, Some(9.0)), (mid, Some(2.0))],
            ),
            (
                "outside the window",
                vec![
                    (start - 5 * HOUR, Some(900.0)),
                    (mid, Some(2.0)),
                    (end + 5 * HOUR, Some(-900.0)),
                ],
            ),
            (
                "all outside",
                vec![(start - 2 * HOUR, Some(1.0)), (start - HOUR, Some(2.0))],
            ),
            (
                "spanning the window",
                vec![(start - HOUR, Some(1.0)), (end + HOUR, Some(5.0))],
            ),
            (
                "extreme timestamps",
                vec![
                    (i64::MIN, Some(1.0)),
                    (i64::MAX, Some(2.0)),
                    (mid, Some(3.0)),
                ],
            ),
        ];
        for (name, points) in cases {
            let mut s = spec(points);
            s.thresholds = vec![
                Threshold {
                    value: f64::NAN,
                    band: Band::Warning,
                    label: "nan".into(),
                },
                Threshold {
                    value: 4.0,
                    band: Band::Warning,
                    label: "Warnung".into(),
                },
                Threshold {
                    value: 6.0,
                    band: Band::Down,
                    label: "Fehler".into(),
                },
            ];
            s.outages = vec![
                (start - HOUR, start + HOUR / 2),
                (end - HOUR, end + HOUR),
                (end + HOUR, end + 2 * HOUR),
            ];
            s.outage_label = Some("keine Antwort".into());
            let png = render_png(&s).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(png_size(&png), (1120, 392), "{name}");
            let svg = render_svg(&s).unwrap();
            assert!(
                !svg.contains("NaN") && !svg.contains("inf"),
                "{name}: {svg}"
            );
        }
    }

    #[test]
    fn invalid_window_is_an_error() {
        let mut s = ping(0);
        s.end_ms = s.start_ms;
        assert!(matches!(render_png(&s), Err(ChartError::InvalidWindow)));
        s.end_ms = s.start_ms - 1;
        assert!(matches!(render_svg(&s), Err(ChartError::InvalidWindow)));
    }

    #[test]
    fn texts_are_escaped() {
        let mut s = ping(5);
        s.unit = "<&>\"'".into();
        s.outages = vec![(window().1 - HOUR, window().1)];
        s.outage_label = Some("a<b & c>d \u{1}\u{8}".into());
        s.thresholds = vec![Threshold {
            value: 3.0,
            band: Band::Down,
            label: "</text><script>&".into(),
        }];
        let svg = render_svg(&s).unwrap();
        assert!(svg.contains("&lt;&amp;&gt;&quot;&apos;"), "{svg}");
        assert!(!svg.contains("<script"));
        assert!(svg.contains("&lt;/text&gt;&lt;script&gt;&amp;"));
        assert!(svg.contains("a&lt;b &amp; c&gt;d"));
        assert!(svg.chars().all(|c| c as u32 >= 0x20 || c == '\n'));
        assert_eq!(png_size(&render_png(&s).unwrap()), (1120, 392));
        assert_eq!(escape("a\u{0}b\tc"), "a b c");
    }

    #[test]
    fn long_texts_are_cut() {
        assert_eq!(clean("  a \n b\u{0} ", 48), "a b");
        let long = "x".repeat(100);
        let cut = clean(&long, 48);
        assert_eq!(cut.chars().count(), 48);
        assert!(cut.ends_with('\u{2026}'));
        assert_eq!(clean("\u{2026}", 1), "\u{2026}");
    }

    #[test]
    fn gaps_break_the_line_and_single_points_become_dots() {
        let (start, end) = window();
        let mut s = spec(vec![
            (start, Some(1.0)),
            (start + HOUR, Some(2.0)),
            (start + 2 * HOUR, None),
            (start + 3 * HOUR, Some(3.0)),
            (start + 4 * HOUR, None),
            (start + 5 * HOUR, Some(4.0)),
            (end - 1000, Some(5.0)),
        ]);
        let shapes = Shapes::from_spec(&s);
        assert_eq!(shapes.lines.len(), 2);
        assert_eq!(shapes.dots.len(), 1);
        // A distance limit breaks the line as well.
        s.points = vec![
            (start, Some(1.0)),
            (start + HOUR, Some(2.0)),
            (start + 4 * HOUR, Some(3.0)),
            (start + 5 * HOUR, Some(3.5)),
        ];
        assert_eq!(Shapes::from_spec(&s).lines.len(), 1);
        s.max_gap_ms = Some(2 * HOUR);
        assert_eq!(Shapes::from_spec(&s).lines.len(), 2);
        let svg = render_svg(&s).unwrap();
        assert_eq!(svg.matches("<circle").count(), 0);
    }

    #[test]
    fn lines_are_cut_at_the_window_edges() {
        let (start, end) = window();
        let s = spec(vec![
            (start - HOUR, Some(0.0)),
            (start + HOUR, Some(10.0)),
            (end - HOUR, Some(10.0)),
            (end + HOUR, Some(0.0)),
        ]);
        let shapes = Shapes::from_spec(&s);
        assert_eq!(shapes.lines.len(), 1);
        let line = &shapes.lines[0];
        assert_eq!(line.first().unwrap(), &(start as f64, 5.0));
        assert_eq!(line.last().unwrap(), &(end as f64, 5.0));
        assert_eq!(line.len(), 4);
        // A point outside on its own is not drawn.
        let s = spec(vec![(start - HOUR, Some(1.0))]);
        let shapes = Shapes::from_spec(&s);
        assert!(shapes.lines.is_empty() && shapes.dots.is_empty());
        // Both neighbours outside: the segment crosses the whole window.
        let s = spec(vec![(start - HOUR, Some(0.0)), (end + HOUR, Some(8.0))]);
        let line = &Shapes::from_spec(&s).lines[0];
        assert_eq!(line.len(), 2);
        assert!(line[0].1 > 0.0 && line[1].1 < 8.0);
    }

    #[test]
    fn nice_steps() {
        assert_eq!(nice_step(0.0, 3.5), 1.0);
        assert_eq!(nice_step(0.0, 10.0), 2.5);
        assert_eq!(nice_step(0.0, 100.0), 25.0);
        assert_eq!(nice_step(0.0, 105.0), 50.0);
        assert_eq!(nice_step(40.0, 100.0), 20.0);
        assert_eq!(nice_step(57.0, 97.0), 20.0);
        assert_eq!(nice_step(0.0, 0.9), 0.25);
        assert_eq!(nice_step(-5.0, 5.0), 2.5);
        assert_eq!(nice_step(0.0, 1.0e6), 250_000.0);
        let tiny = nice_step(0.0, 3e-12);
        assert!((tiny / 1e-12 - 1.0).abs() < 1e-9, "{tiny}");
        // At most four intervals for a lot of ranges, and the range is always covered.
        for lo_i in -20..20 {
            for span_i in 1..60 {
                let (lo, hi) = (lo_i as f64 * 1.7, lo_i as f64 * 1.7 + span_i as f64 * 0.37);
                let axis = build_axis(lo, hi);
                assert!(
                    axis.ticks.len() >= 2 && axis.ticks.len() <= 5,
                    "{lo} {hi} {axis:?}"
                );
                assert!(
                    axis.min <= lo + 1e-9 && axis.max >= hi - 1e-9,
                    "{lo} {hi} {axis:?}"
                );
            }
        }
    }

    #[test]
    fn y_axis_rules() {
        // Ping: from 0, four intervals.
        let a = y_axis(&[1.6, 3.3, 2.0], &[], false);
        assert_eq!((a.min, a.max), (0.0, 4.0));
        assert_eq!(a.ticks, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
        // Temperatures 60..78 with limits at 85 and 95: 40..100, not 0..100.
        let a = y_axis(&[60.0, 66.0, 78.0], &[85.0, 95.0], false);
        assert_eq!((a.min, a.max), (40.0, 100.0));
        // The same without limits does not start at 0 either.
        let a = y_axis(&[60.0, 66.0, 78.0], &[], false);
        assert!(a.min > 0.0 && a.max >= 78.0, "{a:?}");
        // Wide data start at 0.
        let a = y_axis(&[10.0, 90.0], &[], false);
        assert_eq!(a.min, 0.0);
        // Constant data keep the 0 baseline.
        let a = y_axis(&[42.0, 42.0], &[], false);
        assert_eq!(a.min, 0.0);
        assert!(a.max >= 42.0);
        // Only zeros: a range of 0..1; with a limit the limit is shown.
        let a = y_axis(&[0.0, 0.0], &[], false);
        assert_eq!((a.min, a.max), (0.0, 1.0));
        let a = y_axis(&[0.0, 0.0], &[5.0], false);
        assert!(a.max >= 5.0 && a.min == 0.0, "{a:?}");
        // A limit far away is left out.
        let a = y_axis(&[1.6, 3.3], &[100.0], false);
        assert_eq!(a.max, 4.0);
        let a = y_axis(&[1.6, 3.3], &[5.0], false);
        assert!(a.max >= 5.0);
        // Percent ends at 100.
        let a = y_axis(&[20.0, 99.0], &[], true);
        assert_eq!(a.max, 100.0);
        let a = y_axis(&[20.0, 99.0], &[], false);
        assert!(a.max > 100.0);
        let a = y_axis(&[20.0, 120.0], &[], true);
        assert!(a.max > 100.0);
        // Negative values.
        let a = y_axis(&[-8.0, -2.0, 4.0], &[], false);
        assert!(a.min <= -8.0 && a.max >= 4.0 && a.min < 0.0);
        let a = y_axis(&[-70.0, -50.0], &[], false);
        assert!(a.max <= 0.0 + a.step && a.min <= -70.0, "{a:?}");
        // Nothing at all.
        let a = y_axis(&[], &[], false);
        assert_eq!((a.min, a.max), (0.0, 1.0));
        assert_eq!(a.ticks.len(), 5);
        // Only thresholds.
        let a = y_axis(&[], &[80.0, 90.0], false);
        assert!(a.min <= 80.0 && a.max >= 90.0);
    }

    #[test]
    fn tick_labels() {
        let label = |axis: &YAxis, v: f64, comma: bool| TickFormat::for_axis(axis).apply(v, comma);
        let a = build_axis(0.0, 4.0);
        assert_eq!(label(&a, 2.0, false), "2");
        let a = build_axis(0.0, 10.0);
        assert_eq!(label(&a, 7.5, false), "7.5");
        assert_eq!(label(&a, 7.5, true), "7,5");
        assert_eq!(label(&a, 0.0, true), "0");
        assert_eq!(label(&a, 2.5, true), "2,5");
        let a = build_axis(0.0, 0.9);
        assert_eq!(label(&a, 0.25, true), "0,25");
        let a = build_axis(-5.0, 5.0);
        assert_eq!(label(&a, -2.5, false), "-2.5");
        assert_eq!(label(&a, -0.0, false), "0");
        assert_eq!(
            label(&a, -0.04, false),
            "0.0",
            "rounds to zero without a sign"
        );
        let a = build_axis(0.0, 1_500_000.0);
        assert_eq!(label(&a, 1_500_000.0, true), "1,5M");
        let a = build_axis(0.0, 3_000_000.0);
        assert_eq!(label(&a, 2_000_000.0, false), "2M");
        let a = build_axis(0.0, 400_000.0);
        assert_eq!(label(&a, 100_000.0, false), "100000");
        let a = build_axis(0.0, 3e-12);
        assert!(label(&a, 1e-12, false).contains('e'));
        let a = build_axis(0.0, 4e16);
        assert!(label(&a, 1e16, true).contains('e'));
        assert_eq!(decimals_for(1.0), 0);
        assert_eq!(decimals_for(2.5), 1);
        assert_eq!(decimals_for(0.05), 2);
        assert_eq!(decimals_for(0.1 + 0.2), 1);
    }

    #[test]
    fn hour_labels_in_the_time_zone() {
        // 10:00..16:00 in Berlin (CET): seven labels, first and last at the edges.
        let (start, end) = window();
        let ticks = x_ticks(start, end, &berlin());
        let labels: Vec<&str> = ticks.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(
            labels,
            [
                "10:00", "11:00", "12:00", "13:00", "14:00", "15:00", "16:00"
            ]
        );
        assert_eq!(ticks[0].0, start);
        assert_eq!(ticks[6].0, end);
        // A window that does not start at a whole hour: only whole hours inside.
        let ticks = x_ticks(start + 13 * 60_000, end + 13 * 60_000, &berlin());
        let labels: Vec<&str> = ticks.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(
            labels,
            ["11:00", "12:00", "13:00", "14:00", "15:00", "16:00"]
        );
        // UTC and a zone with a half hour offset.
        let ticks = x_ticks(start, end, &TimeZone::UTC);
        assert_eq!(ticks[0].1, "09:00");
        let kolkata = TimeZone::get("Asia/Kolkata").unwrap();
        let ticks = x_ticks(start, end, &kolkata);
        assert_eq!(ticks[0].1, "15:00");
        assert_eq!(ticks.len(), 6);
        // Spring forward in Berlin (2026-03-29, 02:00 -> 03:00): no label for the missing hour.
        let begin = "2026-03-29T00:00:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .as_millisecond();
        let labels: Vec<String> = x_ticks(begin, begin + 6 * HOUR, &berlin())
            .into_iter()
            .map(|t| t.1)
            .collect();
        assert_eq!(
            labels,
            [
                "01:00", "03:00", "04:00", "05:00", "06:00", "07:00", "08:00"
            ]
        );
        // Other window sizes stay readable.
        assert!(x_ticks(start, start + 20 * 60_000, &berlin()).len() <= 9);
        assert!(x_ticks(start, start + 24 * HOUR, &berlin()).len() <= 9);
        assert!(x_ticks(start, start + 24 * HOUR, &berlin()).len() >= 3);
        assert!(x_ticks(start, start + 400 * 24 * HOUR, &berlin()).is_empty());
        assert!(x_ticks(i64::MAX - 5, i64::MAX, &berlin()).is_empty());
    }

    #[test]
    fn outage_label_goes_left_of_the_span_or_aside() {
        let (start, end) = window();
        let mut s = ping(0);
        s.outage_label = Some("12 Min.".into());
        // Late outage: label to the left, ending with an arrow, right-aligned.
        s.outages = vec![(end - HOUR, end)];
        let svg = render_svg(&s).unwrap();
        assert!(svg.contains("12 Min. \u{2192}</text>"), "{svg}");
        assert!(svg.contains(r#"text-anchor="end""#));
        // Outage from the very start: no room on the left, the label goes to the right.
        s.outages = vec![(start - HOUR, start + HOUR)];
        let svg = render_svg(&s).unwrap();
        assert!(svg.contains("\u{2190} 12 Min.</text>"), "{svg}");
        // Outage over the whole window: the plain label inside.
        s.outages = vec![(start - HOUR, end + HOUR)];
        let svg = render_svg(&s).unwrap();
        assert!(svg.contains(">12 Min.</text>"), "{svg}");
        // Without a label or without an outage nothing is added.
        s.outage_label = None;
        assert!(!render_svg(&s).unwrap().contains("12 Min."));
        s.outage_label = Some("12 Min.".into());
        s.outages = vec![];
        assert!(!render_svg(&s).unwrap().contains("12 Min."));
    }

    #[test]
    fn thresholds_outside_the_range_are_not_drawn_and_close_labels_do_not_overlap() {
        let mut s = spec(
            (0..20)
                .map(|i| (window().0 + i * HOUR / 4, Some(60.0 + i as f64)))
                .collect(),
        );
        s.unit = "°C".into();
        s.thresholds = vec![
            Threshold {
                value: 85.0,
                band: Band::Warning,
                label: "Warnung > 85 °C".into(),
            },
            Threshold {
                value: 86.0,
                band: Band::Down,
                label: "Fehler > 86 °C".into(),
            },
            Threshold {
                value: 5000.0,
                band: Band::Down,
                label: "far away".into(),
            },
        ];
        let svg = render_svg(&s).unwrap();
        assert!(!svg.contains("far away"));
        assert!(svg.contains("Warnung &gt; 85 °C") && svg.contains("Fehler &gt; 86 °C"));
        assert_eq!(svg.matches("stroke-dasharray").count(), 2);
        // The labels of two nearby limits end up at different heights.
        let ys: Vec<&str> = svg
            .match_indices(r#"text x=""#)
            .filter_map(|(i, _)| {
                let tail = &svg[i..];
                let end = tail.find("</text>")?;
                (tail[..end].contains("font-weight=\"500\"")).then_some(&tail[..end])
            })
            .collect();
        assert_eq!(ys.len(), 2);
        assert_ne!(ys[0].split('"').nth(3), ys[1].split('"').nth(3), "{ys:?}");
        assert!(render_png(&s).is_ok());
    }

    #[test]
    fn wide_tick_labels_widen_the_left_margin() {
        let (start, _) = window();
        let narrow =
            render_svg(&spec(vec![(start, Some(1.0)), (start + HOUR, Some(3.0))])).unwrap();
        let wide = render_svg(&spec(vec![
            (start, Some(100_000.0)),
            (start + HOUR, Some(450_000.0)),
        ]))
        .unwrap();
        let first_x = |svg: &str| -> f64 {
            let i = svg.find(r#"<path d="M"#).unwrap() + r#"<path d="M"#.len();
            svg[i..].split(',').next().unwrap().parse().unwrap()
        };
        assert_eq!(first_x(&narrow), 44.0);
        assert!(first_x(&wide) > 44.0, "{}", first_x(&wide));
    }

    #[test]
    fn typical_chart_is_small() {
        let (_, end) = window();
        let mut s = ping(8);
        s.outages = vec![(end - 40 * 60_000, end)];
        s.outage_label = Some("keine Antwort".into());
        s.thresholds = vec![Threshold {
            value: 3.0,
            band: Band::Warning,
            label: "Warnung > 3 ms".into(),
        }];
        let png = render_png(&s).unwrap();
        assert!(png.len() < 40_000, "{} bytes", png.len());
    }

    /// Writes example pictures to `target/chart-preview/`:
    /// `cargo test --release chart_png::tests::write_previews -- --ignored --nocapture`.
    #[test]
    #[ignore = "writes files to target/chart-preview/"]
    fn write_previews() {
        let (start, end) = window();
        let n = 72;
        let at = |i: i64| start + (end - start) * i / n;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/chart-preview");
        std::fs::create_dir_all(&dir).unwrap();

        // 1. Ping with an outage of the last 40 minutes.
        let mut ping_outage = ping(8);
        ping_outage.outages = vec![(at(64), end)];
        ping_outage.outage_label = Some("keine Antwort".into());
        ping_outage.thresholds = vec![Threshold {
            value: 3.5,
            band: Band::Warning,
            label: "Warnung > 3,5 ms".into(),
        }];

        // 2. Temperature with two limits, rising.
        let temp = |i: i64| {
            61.0 + 2.0 * (i as f64 / 10.0).sin()
                + (i as f64 * 0.37).sin().abs() * 1.5
                + (i - 48).max(0) as f64 * 0.62
        };
        let mut temperature = spec((0..n).map(|i| (at(i), Some(temp(i)))).collect());
        temperature.unit = "°C".into();
        temperature.colour = Colour::Secondary;
        temperature.thresholds = vec![
            Threshold {
                value: 85.0,
                band: Band::Warning,
                label: "Warnung > 85 °C".into(),
            },
            Threshold {
                value: 95.0,
                band: Band::Down,
                label: "Fehler > 95 °C".into(),
            },
        ];

        // 3. Recovery after a short outage in the middle.
        let mut recovery = ping(0);
        for p in &mut recovery.points[40..43] {
            p.1 = None;
        }
        recovery.outages = vec![(at(40), at(43))];
        recovery.outage_label = Some("12 Min.".into());

        // 4. English, wide numbers, percent and a dot.
        let mut load = spec(
            (0..n)
                .map(|i| {
                    (
                        at(i),
                        (i % 17 != 5).then_some(20.0 + 60.0 * ((i as f64) / 11.0).sin().abs()),
                    )
                })
                .collect(),
        );
        load.unit = "%".into();
        load.decimal_comma = false;
        load.thresholds = vec![Threshold {
            value: 90.0,
            band: Band::Down,
            label: "Error > 90 %".into(),
        }];

        // 5. Edge cases to look at: one dot, negative values, wide numbers.
        let mut dot = spec(vec![(at(30), Some(2.5))]);
        dot.thresholds = vec![Threshold {
            value: 3.0,
            band: Band::Warning,
            label: "Warnung > 3 ms".into(),
        }];
        let mut negative = spec(
            (0..n)
                .map(|i| (at(i), Some(-70.0 + 15.0 * (i as f64 / 9.0).sin())))
                .collect(),
        );
        negative.unit = "dBm".into();
        let mut wide = spec(
            (0..n)
                .map(|i| {
                    (
                        at(i),
                        Some(120_000.0 + 1_900_000.0 * (i as f64 / 20.0).sin().abs()),
                    )
                })
                .collect(),
        );
        wide.unit = "B/s".into();
        wide.outages = vec![(at(0), at(8))];
        wide.outage_label = Some("keine Antwort".into());

        let mut total = 0;
        for (name, chart) in [
            ("ping-outage", ping_outage),
            ("temperature", temperature),
            ("recovery", recovery),
            ("load-percent", load),
            ("edge-dot", dot),
            ("edge-negative", negative),
            ("edge-wide", wide),
        ] {
            let timer = std::time::Instant::now();
            let png = render_png(&chart).unwrap();
            let first = timer.elapsed();
            let timer = std::time::Instant::now();
            for _ in 0..20 {
                render_png(&chart).unwrap();
            }
            let warm = timer.elapsed() / 20;
            let file = dir.join(format!("{name}.png"));
            std::fs::write(&file, &png).unwrap();
            std::fs::write(dir.join(format!("{name}.svg")), render_svg(&chart).unwrap()).unwrap();
            println!(
                "{} {} bytes, first render {first:?}, average of 20 more {warm:?}",
                file.display(),
                png.len()
            );
            total += png.len();
        }
        assert!(total > 0);
    }
}
