//! Values of a router in front of the network (admin only): like "Werte dieses Servers" a page with
//! what the device can report and the current values, to create the ticked ones as sensors. The
//! values come from the device's address; opening the page asks the router once, read-only.

use std::collections::HashSet;
use std::time::Duration;

use askama::Template;
use axum::Form;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};

use crate::devices::Device;
use crate::error::{Error, Result};
use crate::i18n::{Lang, Msg};
use crate::sensors::PollContext;
use crate::sensors::format::format_value;
use crate::sensors::fritzbox::{FritzCheck, FritzMetric};
use crate::sensors::huawei::{HuaweiCheck, HuaweiMetric};
use crate::sensors::ping::PingClients;
use crate::sensors::spec::SensorSpec;
use crate::status::shown;
use crate::storage::{devices, sensors as store};
use crate::web::auth::CurrentUser;
use crate::web::sensors::now_ms;
use crate::web::server_values::create_named;
use crate::web::{AppState, Crumb, Layout, group_trail};

/// Upper bound of fields in one "create" form.
const MAX_FIELDS: usize = 20;

/// The routers with a page of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Fritzbox,
    Huawei,
}

/// A value of a router that can become a sensor.
struct Candidate {
    /// The form's value.
    metric: &'static str,
    /// Message ids of the name and the description.
    name: &'static str,
    detail: &'static str,
    spec: SensorSpec,
}

impl Family {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "fritzbox" => Some(Self::Fritzbox),
            "huawei" => Some(Self::Huawei),
            _ => None,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Fritzbox => "fritzbox",
            Self::Huawei => "huawei",
        }
    }

    pub fn href(self, device: i64) -> String {
        format!("/devices/{device}/router/{}", self.id())
    }

    fn candidates(self) -> Vec<Candidate> {
        let fritzbox = |metric, name, detail, m| Candidate {
            metric,
            name,
            detail,
            spec: SensorSpec::Fritzbox(FritzCheck::new(m)),
        };
        let huawei = |metric, name, detail, m| Candidate {
            metric,
            name,
            detail,
            spec: SensorSpec::Huawei(HuaweiCheck::new(m)),
        };
        match self {
            Self::Fritzbox => vec![
                fritzbox(
                    "connection",
                    "fritzbox-name-connection",
                    "fritzbox-detail-connection",
                    FritzMetric::Connection,
                ),
                fritzbox(
                    "traffic",
                    "fritzbox-name-traffic",
                    "fritzbox-detail-traffic",
                    FritzMetric::Traffic,
                ),
            ],
            Self::Huawei => vec![
                huawei(
                    "connection",
                    "huawei-name-connection",
                    "huawei-detail-connection",
                    HuaweiMetric::Connection,
                ),
                huawei(
                    "traffic",
                    "huawei-name-traffic",
                    "huawei-detail-traffic",
                    HuaweiMetric::Traffic,
                ),
                huawei(
                    "volume",
                    "huawei-name-volume",
                    "huawei-detail-volume",
                    HuaweiMetric::Volume,
                ),
            ],
        }
    }
}

struct CandidateView {
    metric: &'static str,
    name: String,
    detail: String,
    value: String,
    exists: bool,
}

#[derive(Template)]
#[template(path = "router_values.html")]
struct PageTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    device: Device,
    /// Groups above the device, for the breadcrumbs.
    trail: Vec<Crumb>,
    family: &'static str,
    candidates: Vec<CandidateView>,
    /// Why the router did not answer.
    problem: Option<String>,
}

async fn load_device(state: &AppState, id: i64) -> Result<Device> {
    devices::get_device(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)
}

/// The value of `spec` now, as one poll of a fresh sensor sees it. Counters need two polls, so
/// traffic shows none yet.
async fn current(spec: &SensorSpec, device: &Device, t: Lang) -> std::result::Result<String, Msg> {
    let ping = PingClients::new();
    let mut ctx = PollContext {
        host: &device.host,
        now_ms: now_ms(),
        timeout: Duration::from_secs(u64::from(device.polling.timeout_secs)),
        ping: &ping,
        snmp: None,
    };
    let mut sensor = spec.build();
    let measured = sensor.poll(&mut ctx).await.map_err(|e| e.user_message())?;
    let values: Vec<String> = measured
        .channels
        .iter()
        .filter(|c| spec.in_lists(&c.key))
        .filter_map(|c| c.value.map(|v| format_value(t, c.unit, v)))
        .collect();
    Ok(if values.is_empty() {
        "–".to_owned()
    } else {
        values.join(" · ")
    })
}

fn family(text: &str) -> Result<Family> {
    Family::parse(text).ok_or(Error::NotFound)
}

async fn existing_specs(state: &AppState, device: i64) -> Result<Vec<SensorSpec>> {
    Ok(store::list_sensors(&state.pool, device)
        .await?
        .into_iter()
        .map(|s| s.spec)
        .collect())
}

pub async fn page(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, family_id)): Path<(i64, String)>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let family = family(&family_id)?;
    let device = load_device(&state, id).await?;
    let existing = existing_specs(&state, id).await?;
    let t = user.lang;
    let mut problem = None;
    let mut candidates = Vec::new();
    for c in family.candidates() {
        // After the first failure the others would only wait for the same timeout.
        let value = if problem.is_some() {
            "–".to_owned()
        } else {
            match current(&c.spec, &device, t).await {
                Ok(value) => value,
                Err(message) => {
                    problem = Some(shown(t, &message));
                    "–".to_owned()
                }
            }
        };
        candidates.push(CandidateView {
            exists: existing.contains(&c.spec),
            metric: c.metric,
            name: t.get(c.name),
            detail: t.get(c.detail),
            value,
        });
    }
    let page = PageTemplate {
        layout: user.layout(),
        t,
        trail: group_trail(&state.pool, device.group_id).await?,
        device,
        family: family.id(),
        candidates,
        problem,
    };
    Ok(Html(page.render()?))
}

/// Creates the ticked values (fields `metric=<metric>`), with default limits.
pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, family_id)): Path<(i64, String)>,
    Form(fields): Form<Vec<(String, String)>>,
) -> Result<Response> {
    user.require_admin()?;
    let csrf = fields
        .iter()
        .find(|(k, _)| k == "csrf_token")
        .map_or("", |(_, v)| v.as_str());
    user.verify_csrf(csrf)?;
    let family = family(&family_id)?;
    let device = load_device(&state, id).await?;
    if fields.len() > MAX_FIELDS {
        return Err(Error::invalid(Msg::new("form-too-large")));
    }
    let wanted: HashSet<&str> = fields
        .iter()
        .filter(|(k, _)| k == "metric")
        .map(|(_, v)| v.as_str())
        .collect();
    if wanted.is_empty() {
        return Err(Error::invalid(Msg::new("local-select-value")));
    }
    let existing = existing_specs(&state, id).await?;
    let mut created = 0usize;
    for c in family.candidates() {
        if !wanted.contains(c.metric) || existing.contains(&c.spec) {
            continue;
        }
        let name = user.lang.get(c.name);
        let sensor = create_named(&state, id, &name, &c.spec).await?;
        let name = store::get_sensor(&state.pool, sensor)
            .await?
            .map_or(name, |s| s.name);
        crate::web::sensors::audit_created(&state, &user, sensor, &name, &device).await;
        created += 1;
    }
    tracing::info!(user = %user.username, device = id, created, "router sensors created");
    Ok(Redirect::to(&format!("/devices/{id}/sensors?msg=sensors_added")).into_response())
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;
    use crate::devices::{DeviceInput, DeviceKind, Polling};
    use crate::http_client::testserver::{ok, serve};
    use crate::sensors::fritzbox;
    use crate::storage::devices::save_device;
    use crate::web::testutil::*;

    async fn router(h: &Harness, host: &str) -> i64 {
        save_device(
            &h.state.pool,
            None,
            DeviceInput {
                kind: DeviceKind::Router,
                name: "Router".to_owned(),
                host: host.to_owned(),
                group_id: None,
                profile_id: None,
                polling: Polling {
                    timeout_secs: 1,
                    ..Polling::default()
                },
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn the_add_form_leads_to_the_page_of_the_router() {
        let h = harness(true).await;
        let device = router(&h, "192.0.2.30").await;
        let (admin, csrf) = login(&h, "anna").await;
        let list = send(
            &h.app,
            get(&format!("/devices/{device}/sensors"), Some(&admin)),
        )
        .await;
        assert!(list.body.contains("<option value=\"fritzbox\">"));
        assert!(list.body.contains("<option value=\"huawei\">"));
        for kind in ["fritzbox", "huawei"] {
            let reply = send(
                &h.app,
                post(
                    &format!("/devices/{device}/sensors"),
                    Some(&admin),
                    &format!("csrf_token={csrf}&kind={kind}&name="),
                ),
            )
            .await;
            assert_eq!(reply.status, StatusCode::SEE_OTHER);
            assert_eq!(reply.location(), format!("/devices/{device}/router/{kind}"));
        }
    }

    #[tokio::test]
    async fn creates_the_ticked_values_once_and_marks_them_as_present() {
        let h = harness(true).await;
        // Nothing listens here: the page says so and still offers the values.
        let device = router(&h, "127.0.0.1").await;
        let (admin, csrf) = login(&h, "anna").await;
        let path = format!("/devices/{device}/router/huawei");
        let form = format!("csrf_token={csrf}&metric=connection&metric=volume&metric=unknown");
        let done = send(&h.app, post(&path, Some(&admin), &form)).await;
        assert_eq!(done.status, StatusCode::SEE_OTHER);
        send(&h.app, post(&path, Some(&admin), &form)).await;
        let sensors = store::list_sensors(&h.state.pool, device).await.unwrap();
        assert_eq!(sensors.len(), 2, "unknown and repeated values add nothing");
        let connection = sensors
            .iter()
            .find(|s| s.spec == SensorSpec::Huawei(HuaweiCheck::new(HuaweiMetric::Connection)))
            .unwrap();
        assert_eq!(connection.name, "Mobilfunkverbindung");
        let signal = connection.channels.iter().find(|c| c.key == "signal");
        assert_eq!(signal.unwrap().thresholds.warn_below, Some(2.0));

        let page = send(&h.app, get(&path, Some(&admin))).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains("class=\"error\""), "{}", page.body);
        let box_ = page.body.split("value=\"volume\"").nth(1).unwrap();
        assert!(box_[..box_.find('>').unwrap()].contains("disabled checked"));
        let detail = send(
            &h.app,
            get(&format!("/sensors/{}", connection.id), Some(&admin)),
        )
        .await;
        assert!(detail.body.contains("Mobilfunkverbindung"));
    }

    #[tokio::test]
    async fn the_page_shows_what_the_router_answers() {
        let h = harness(true).await;
        let status =
            "<NewConnectionStatus>Connected</NewConnectionStatus><NewUptime>60</NewUptime>";
        let addon = "<NewX_AVM_DE_TotalBytesSent64>1</NewX_AVM_DE_TotalBytesSent64>\
            <NewX_AVM_DE_TotalBytesReceived64>2</NewX_AVM_DE_TotalBytesReceived64>\
            <NewX_AVM_DE_Layer1UpstreamMaxBitRate64>50000000</NewX_AVM_DE_Layer1UpstreamMaxBitRate64>\
            <NewX_AVM_DE_Layer1DownstreamMaxBitRate64>1000000000</NewX_AVM_DE_Layer1DownstreamMaxBitRate64>";
        let addr = serve(vec![ok(status), ok(addon), ok(addon)], None).await;
        let spec = SensorSpec::Fritzbox(FritzCheck {
            metric: FritzMetric::Connection,
            port: addr.port(),
        });
        let device = Device {
            id: 1,
            name: "x".into(),
            host: "127.0.0.1".into(),
            group_id: None,
            profile_id: None,
            profile_name: None,
            polling: Polling::default(),
            kind: DeviceKind::Router,
        };
        let value = current(&spec, &device, Lang::En).await.unwrap();
        assert_eq!(value, "1 Gbit/s · 50 Mbit/s");
        assert_eq!(fritzbox::DEFAULT_PORT, 49000);
        drop(h);
    }

    #[tokio::test]
    async fn only_admins_with_a_valid_token_may_use_it() {
        let h = harness(true).await;
        let device = router(&h, "127.0.0.1").await;
        let path = format!("/devices/{device}/router/fritzbox");
        let (viewer, vcsrf) = login(&h, "bert").await;
        assert_eq!(
            send(&h.app, get(&path, Some(&viewer))).await.status,
            StatusCode::FORBIDDEN
        );
        let form = format!("csrf_token={vcsrf}&metric=traffic");
        assert_eq!(
            send(&h.app, post(&path, Some(&viewer), &form)).await.status,
            StatusCode::FORBIDDEN
        );
        let (admin, csrf) = login(&h, "anna").await;
        let bad = send(
            &h.app,
            post(&path, Some(&admin), "csrf_token=wrong&metric=traffic"),
        )
        .await;
        assert_eq!(bad.status, StatusCode::FORBIDDEN);
        let none = send(
            &h.app,
            post(&path, Some(&admin), &format!("csrf_token={csrf}")),
        )
        .await;
        assert_eq!(none.status, StatusCode::UNPROCESSABLE_ENTITY);
        let other = format!("/devices/{device}/router/nothing");
        assert_eq!(
            send(&h.app, get(&other, Some(&admin))).await.status,
            StatusCode::NOT_FOUND
        );
        assert!(
            store::list_sensors(&h.state.pool, device)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
