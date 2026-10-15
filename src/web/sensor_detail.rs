//! Sensor detail page: current values, chart (uPlot, data from a JSON endpoint), thresholds, events.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::{Form, Json};
use serde::{Deserialize, Serialize};

use crate::devices::Device;
use crate::error::{Error, Result};
use crate::sensors::Unit;
use crate::sensors::format::{
    channel_label, format_value, input_unit, parse_threshold, threshold_input,
};
use crate::status::{Status, Thresholds};
use crate::storage::devices;
use crate::storage::measurements::{Range, chart_data, latest_values};
use crate::storage::sensors::{self as store, ChannelRecord, SensorRecord};
use crate::storage::status as status_store;
use crate::web::auth::CurrentUser;
use crate::web::{AppState, Layout, flash_message, format_utc};

const EVENT_LIMIT: u32 = 20;

#[derive(Deserialize)]
pub struct RangeQuery {
    range: Option<String>,
    msg: Option<String>,
}

impl RangeQuery {
    fn range(&self) -> Range {
        self.range
            .as_deref()
            .and_then(Range::parse)
            .unwrap_or(Range::Day)
    }
}

struct ChannelView {
    id: i64,
    label: String,
    latest: String,
    latest_at: String,
    /// Threshold inputs: warn above, warn below, down above, down below (in `input_unit`).
    fields: [String; 4],
    unit_label: &'static str,
    has_thresholds: bool,
}

struct RangeLink {
    value: &'static str,
    label: &'static str,
    current: bool,
}

struct EventView {
    at: String,
    change: String,
    message: String,
}

#[derive(Template)]
#[template(path = "sensor_detail.html")]
struct DetailTemplate {
    layout: Layout,
    sensor: SensorRecord,
    description: String,
    device: Device,
    status_label: &'static str,
    status_class: &'static str,
    message: String,
    last_poll: String,
    channels: Vec<ChannelView>,
    ranges: Vec<RangeLink>,
    range: &'static str,
    events: Vec<EventView>,
    flash: Option<&'static str>,
    error: Option<String>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

async fn load(state: &AppState, id: i64) -> Result<(SensorRecord, Device)> {
    let sensor = store::get_sensor(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)?;
    let device = devices::get_device(&state.pool, sensor.device_id)
        .await?
        .ok_or(Error::NotFound)?;
    Ok((sensor, device))
}

fn fields_of(t: &Thresholds, unit: Unit) -> [String; 4] {
    [t.warn_above, t.warn_below, t.down_above, t.down_below].map(|v| threshold_input(v, unit))
}

/// Variable parts of the page.
struct View<'a> {
    range: Range,
    flash: Option<&'static str>,
    error: Option<String>,
    /// Rejected threshold input to echo back (by field name).
    submitted: Option<&'a HashMap<String, String>>,
}

async fn render(
    state: &AppState,
    user: &CurrentUser,
    sensor: SensorRecord,
    device: Device,
    view: View<'_>,
) -> Result<String> {
    let View {
        range,
        flash,
        error,
        submitted,
    } = view;
    let latest = latest_values(&state.pool, &sensor.channels).await?;
    let channels = sensor
        .channels
        .iter()
        .map(|c| {
            let fields = match submitted {
                Some(form) => ["wa", "wb", "da", "db"].map(|p| {
                    form.get(&format!("{p}_{}", c.id))
                        .cloned()
                        .unwrap_or_default()
                }),
                None => fields_of(&c.thresholds, c.unit),
            };
            ChannelView {
                id: c.id,
                label: channel_label(&c.key).to_owned(),
                latest: latest
                    .get(&c.id)
                    .map_or_else(|| "–".to_owned(), |(_, v)| format_value(c.unit, *v)),
                latest_at: latest
                    .get(&c.id)
                    .map_or_else(String::new, |(ts, _)| format_utc(*ts)),
                fields,
                unit_label: input_unit(c.unit),
                has_thresholds: c.unit != Unit::State,
            }
        })
        .collect();
    let state_row = status_store::get_state(&state.pool, sensor.id).await?;
    let status = if sensor.enabled {
        state_row.as_ref().map_or(Status::Unknown, |s| s.status)
    } else {
        Status::Paused
    };
    let events = status_store::events_of_sensor(&state.pool, sensor.id, EVENT_LIMIT)
        .await?
        .into_iter()
        .map(|e| EventView {
            at: format_utc(e.ts_ms),
            change: match e.from {
                Some(from) => format!("{} → {}", from.label(), e.to.label()),
                None => format!("erste Abfrage: {}", e.to.label()),
            },
            message: e.message,
        })
        .collect();
    let page = DetailTemplate {
        layout: user.layout(),
        description: sensor.spec.describe(),
        status_label: status.label(),
        status_class: status.as_str(),
        message: state_row
            .as_ref()
            .map(|s| s.message.clone())
            .unwrap_or_default(),
        last_poll: state_row.map_or_else(String::new, |s| format_utc(s.last_poll_ms)),
        channels,
        ranges: Range::ALL
            .into_iter()
            .map(|r| RangeLink {
                value: r.as_str(),
                label: r.label(),
                current: r == range,
            })
            .collect(),
        range: range.as_str(),
        events,
        flash,
        error,
        sensor,
        device,
    };
    Ok(page.render()?)
}

pub async fn detail(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<RangeQuery>,
) -> Result<Html<String>> {
    let (sensor, device) = load(&state, id).await?;
    Ok(Html(
        render(
            &state,
            &user,
            sensor,
            device,
            View {
                range: query.range(),
                flash: flash_message(query.msg.as_deref()),
                error: None,
                submitted: None,
            },
        )
        .await?,
    ))
}

#[derive(Serialize)]
struct SeriesJson {
    key: String,
    label: String,
    unit: &'static str,
    values: Vec<Option<f64>>,
}

#[derive(Serialize)]
pub struct ChartJson {
    range: &'static str,
    ts: Vec<i64>,
    series: Vec<SeriesJson>,
}

/// Chart data for uPlot: aligned timestamps (seconds) and one value list per channel.
pub async fn data(
    State(state): State<AppState>,
    _user: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<RangeQuery>,
) -> Result<Json<ChartJson>> {
    let sensor = store::get_sensor(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)?;
    let range = query.range();
    let chart = chart_data(&state.pool, &sensor.channels, range, now_ms()).await?;
    Ok(Json(ChartJson {
        range: range.as_str(),
        ts: chart.ts,
        series: chart
            .series
            .into_iter()
            .map(|s| SeriesJson {
                label: channel_label(&s.key).to_owned(),
                unit: s.unit.as_str(),
                key: s.key,
                values: s.values,
            })
            .collect(),
    }))
}

/// Parses the threshold fields of every channel from the submitted form.
fn parse_limits(
    channels: &[ChannelRecord],
    form: &HashMap<String, String>,
) -> Result<Vec<(i64, Thresholds)>> {
    channels
        .iter()
        .filter(|c| c.unit != Unit::State)
        .map(|c| {
            let field = |prefix: &str| {
                let text = form
                    .get(&format!("{prefix}_{}", c.id))
                    .map_or("", String::as_str);
                parse_threshold(text, c.unit).map_err(|m| Error::Invalid(m.to_owned()))
            };
            let limits = Thresholds {
                warn_above: field("wa")?,
                warn_below: field("wb")?,
                down_above: field("da")?,
                down_below: field("db")?,
            };
            limits
                .validate()
                .map_err(|m| Error::Invalid(format!("{}: {m}", channel_label(&c.key))))?;
            Ok((c.id, limits))
        })
        .collect()
}

pub async fn save_thresholds(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(form.get("csrf_token").map_or("", String::as_str))?;
    let (sensor, device) = load(&state, id).await?;
    match parse_limits(&sensor.channels, &form) {
        Ok(limits) => {
            store::set_thresholds(&state.pool, id, &limits).await?;
            tracing::info!(user = %user.username, sensor = id, "thresholds saved");
            Ok(Redirect::to(&format!("/sensors/{id}?msg=thresholds_saved")).into_response())
        }
        Err(Error::Invalid(message)) => {
            let html = render(
                &state,
                &user,
                sensor,
                device,
                View {
                    range: Range::Day,
                    flash: None,
                    error: Some(message),
                    submitted: Some(&form),
                },
            )
            .await?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response())
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{DeviceInput, Polling};
    use crate::sensors::spec::SensorSpec;
    use crate::status::Assessment;
    use crate::storage::devices::save_device;
    use crate::storage::sensors::{Sample, create_sensor, get_sensor, insert_samples};
    use crate::storage::status::record_poll;
    use crate::web::testutil::*;
    use axum::http::header;

    /// A traffic sensor with samples; returns (sensor id, [in channel, out channel]).
    async fn traffic_sensor(h: &Harness) -> (i64, Vec<i64>) {
        let profile = add_v2c_profile(h, "P", "public").await;
        let device = save_device(
            &h.state.pool,
            None,
            DeviceInput {
                name: "Switch".to_owned(),
                host: "192.0.2.1".to_owned(),
                group_id: None,
                profile_id: profile,
                polling: Polling::default(),
            },
        )
        .await
        .unwrap();
        let sensor = create_sensor(
            &h.state.pool,
            device,
            "Traffic Port 1",
            &SensorSpec::InterfaceTraffic { if_index: 49153 },
        )
        .await
        .unwrap();
        let channels: Vec<i64> = get_sensor(&h.state.pool, sensor)
            .await
            .unwrap()
            .unwrap()
            .channels
            .iter()
            .map(|c| c.id)
            .collect();
        let now = now_ms();
        let samples = [
            Sample {
                channel_id: channels[0],
                ts_ms: now - 20_000,
                value: 12_500_000.0,
            },
            Sample {
                channel_id: channels[1],
                ts_ms: now - 20_000,
                value: 3_000.0,
            },
        ];
        insert_samples(&h.state.pool, &samples).await.unwrap();
        record_poll(
            &h.state.pool,
            sensor,
            now - 20_000,
            &[],
            &Assessment::Healthy("OK".to_owned()),
        )
        .await
        .unwrap();
        (sensor, channels)
    }

    #[tokio::test]
    async fn the_page_shows_latest_values_chart_hooks_and_events() {
        let h = harness(true).await;
        let (sensor, _) = traffic_sensor(&h).await;
        let (viewer, _) = login(&h, "bert").await;
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&viewer))).await;
        assert_eq!(page.status, StatusCode::OK);
        for needle in [
            "Traffic Port 1",
            "Eingehend",
            "12,5 Mbit/s",
            "3 kbit/s",
            "In Ordnung",
            "erste Abfrage: In Ordnung",
            "data-src=\"/sensors/1/data?range=24h\"",
            "/static/vendor/uplot/uPlot.iife.min.js",
            "/static/chart.js",
        ] {
            assert!(
                page.body.contains(needle),
                "{needle} missing: {}",
                page.body
            );
        }
        assert!(
            !page.body.contains("name=\"csrf_token\"") || !page.body.contains("/thresholds"),
            "viewers get no form"
        );
        assert!(page.body.contains("nur Administratoren"));

        let week = send(
            &h.app,
            get(&format!("/sensors/{sensor}?range=7d"), Some(&viewer)),
        )
        .await;
        assert!(week.body.contains("range=7d\"") && week.body.contains("aria-current=\"page\""));
        let bogus = send(
            &h.app,
            get(&format!("/sensors/{sensor}?range=evil"), Some(&viewer)),
        )
        .await;
        assert!(
            bogus.body.contains("range=24h\""),
            "invalid ranges fall back to 24 hours"
        );
    }

    #[tokio::test]
    async fn the_json_endpoint_serves_aligned_series() {
        let h = harness(true).await;
        let (sensor, _) = traffic_sensor(&h).await;
        let (viewer, _) = login(&h, "bert").await;
        let reply = send(
            &h.app,
            get(&format!("/sensors/{sensor}/data?range=1h"), Some(&viewer)),
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK);
        assert!(
            reply.headers[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("application/json")
        );
        let body = &reply.body;
        assert!(body.contains("\"range\":\"1h\""), "{body}");
        assert!(
            body.contains("\"label\":\"Eingehend\"") && body.contains("\"unit\":\"bps\""),
            "{body}"
        );
        assert!(
            body.contains("12500000.0") && body.contains("3000.0"),
            "{body}"
        );

        assert_eq!(
            send(&h.app, get(&format!("/sensors/{sensor}/data"), None))
                .await
                .status,
            StatusCode::SEE_OTHER
        );
        assert_eq!(
            send(&h.app, get("/sensors/99/data", Some(&viewer)))
                .await
                .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(&h.app, get("/sensors/99", Some(&viewer))).await.status,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn admins_save_thresholds_in_the_display_unit() {
        let h = harness(true).await;
        let (sensor, channels) = traffic_sensor(&h).await;
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&admin))).await;
        assert!(
            page.body.contains(&format!("name=\"wa_{}\"", channels[0]))
                && page.body.contains("in Mbit/s")
        );

        let body = format!(
            "csrf_token={csrf}&wa_{0}=80%2C5&da_{0}=95&wb_{0}=&db_{0}=&wa_{1}=&wb_{1}=&da_{1}=&db_{1}=",
            channels[0], channels[1]
        );
        let reply = send(
            &h.app,
            post(
                &format!("/sensors/{sensor}/thresholds"),
                Some(&admin),
                &body,
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/sensors/{sensor}?msg=thresholds_saved")
        );
        let stored = get_sensor(&h.state.pool, sensor).await.unwrap().unwrap();
        assert_eq!(stored.channels[0].thresholds.warn_above, Some(80_500_000.0));
        assert_eq!(stored.channels[0].thresholds.down_above, Some(95_000_000.0));
        assert!(stored.channels[1].thresholds.is_empty());

        let page = send(
            &h.app,
            get(
                &format!("/sensors/{sensor}?msg=thresholds_saved"),
                Some(&admin),
            ),
        )
        .await;
        assert!(
            page.body.contains("Schwellwerte gespeichert.") && page.body.contains("value=\"80,5\"")
        );
    }

    #[tokio::test]
    async fn bad_thresholds_are_explained_and_nothing_is_saved() {
        let h = harness(true).await;
        let (sensor, channels) = traffic_sensor(&h).await;
        let (admin, csrf) = login(&h, "anna").await;
        let path = format!("/sensors/{sensor}/thresholds");
        for (fields, expected) in [
            ("wa_{0}=abc".to_owned(), "Zahl"),
            ("wa_{0}=90&da_{0}=50".to_owned(), "Down-Grenze"),
            ("wa_{0}=10&wb_{0}=20".to_owned(), "unteren Grenze"),
        ] {
            let body = format!(
                "csrf_token={csrf}&{}",
                fields.replace("{0}", &channels[0].to_string())
            );
            let reply = send(&h.app, post(&path, Some(&admin), &body)).await;
            assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
            assert!(reply.body.contains(expected), "{body}: {}", reply.body);
            assert!(reply.body.contains("Eingehend:") || expected == "Zahl");
        }
        let echoed = send(
            &h.app,
            post(
                &path,
                Some(&admin),
                &format!("csrf_token={csrf}&wa_{}=abc", channels[0]),
            ),
        )
        .await;
        assert!(
            echoed.body.contains("value=\"abc\""),
            "input stays in the form"
        );
        let stored = get_sensor(&h.state.pool, sensor).await.unwrap().unwrap();
        assert!(stored.channels.iter().all(|c| c.thresholds.is_empty()));
    }

    #[tokio::test]
    async fn only_admins_with_a_valid_token_may_save_thresholds() {
        let h = harness(true).await;
        let (sensor, channels) = traffic_sensor(&h).await;
        let path = format!("/sensors/{sensor}/thresholds");
        let (viewer, vcsrf) = login(&h, "bert").await;
        let body = |csrf: &str| format!("csrf_token={csrf}&wa_{}=1", channels[0]);
        assert_eq!(
            send(&h.app, post(&path, Some(&viewer), &body(&vcsrf)))
                .await
                .status,
            StatusCode::FORBIDDEN
        );
        let (admin, _) = login(&h, "anna").await;
        assert_eq!(
            send(&h.app, post(&path, Some(&admin), &body("wrong")))
                .await
                .status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(&h.app, post(&path, None, &body("x"))).await.status,
            StatusCode::SEE_OTHER
        );
        let stored = get_sensor(&h.state.pool, sensor).await.unwrap().unwrap();
        assert!(stored.channels.iter().all(|c| c.thresholds.is_empty()));
    }

    #[tokio::test]
    async fn status_channels_get_no_threshold_inputs() {
        let h = harness(true).await;
        let profile = add_v2c_profile(&h, "P", "public").await;
        let device = save_device(
            &h.state.pool,
            None,
            DeviceInput {
                name: "AP".to_owned(),
                host: "192.0.2.2".to_owned(),
                group_id: None,
                profile_id: profile,
                polling: Polling::default(),
            },
        )
        .await
        .unwrap();
        let sensor = create_sensor(
            &h.state.pool,
            device,
            "Port",
            &SensorSpec::InterfaceStatus { if_index: 1 },
        )
        .await
        .unwrap();
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(&h.app, get(&format!("/sensors/{sensor}"), Some(&admin))).await;
        assert!(!page.body.contains("name=\"wa_"));
        let reply = send(
            &h.app,
            post(
                &format!("/sensors/{sensor}/thresholds"),
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(
            reply.status,
            StatusCode::SEE_OTHER,
            "an empty form is a valid no-op"
        );
    }

    #[tokio::test]
    async fn chart_assets_are_served_from_the_binary() {
        let h = harness(true).await;
        for (path, content) in [
            ("/static/vendor/uplot/uPlot.iife.min.js", "javascript"),
            ("/static/vendor/uplot/uPlot.min.css", "css"),
            ("/static/chart.js", "javascript"),
            ("/static/vendor/uplot/LICENSE", ""),
        ] {
            let reply = send(&h.app, get(path, None)).await;
            assert_eq!(reply.status, StatusCode::OK, "{path}");
            assert!(
                reply.headers[header::CONTENT_TYPE]
                    .to_str()
                    .unwrap()
                    .contains(content),
                "{path}"
            );
            assert!(!reply.body.is_empty());
        }
    }
}
