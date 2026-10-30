//! "Werte dieses Servers" (admin only): lists what the machine opennetmon runs on can report
//! (CPU, load, memory, uptime, file systems, temperatures, network interfaces) and creates the
//! ticked values as local sensors of a device. The values always come from this machine, whatever
//! the device's address is.

use std::collections::HashSet;

use askama::Template;
use axum::Form;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};

use crate::devices::{Device, DeviceKind};
use crate::error::{Error, Result};
use crate::i18n::{Lang, Msg};
use crate::sensors::Unit;
use crate::sensors::format::format_value;
use crate::sensors::local::{self, LocalMetric, Paths};
use crate::sensors::spec::SensorSpec;
use crate::status::Thresholds;
use crate::storage::devices;
use crate::storage::sensors as store;
use crate::web::auth::CurrentUser;
use crate::web::{AppState, Crumb, Layout, group_trail};

const MAX_NAME_CHARS: usize = 64;
/// Upper bound of fields in one "create" form.
const MAX_FIELDS: usize = 2_000;

/// Whether the sensor page offers this page: devices of type server, devices addressed as this
/// machine, and devices that already have local sensors.
pub fn offered(device: &Device, has_local: bool) -> bool {
    let host = device.host.trim().to_ascii_lowercase();
    device.kind == DeviceKind::Server
        || has_local
        || host == "localhost"
        || host == "::1"
        || host.starts_with("127.")
}

/// One value that can become a sensor.
struct Candidate {
    metric: LocalMetric,
    /// Sensor name in the admin's language, e.g. `Dateisystem /var`.
    name: String,
    detail: String,
    /// Current value, if it can be read in one go.
    value: String,
    preselect: bool,
    /// Limits set on creation (in addition to [`LocalMetric::default_limits`]).
    limits: Vec<(&'static str, Thresholds)>,
}

struct CandidateView {
    target: String,
    name: String,
    detail: String,
    value: String,
    selected: bool,
    exists: bool,
}

struct SectionView {
    title: String,
    candidates: Vec<CandidateView>,
}

#[derive(Template)]
#[template(path = "server_values.html")]
struct PageTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    device: Device,
    /// Groups above the device, for the breadcrumbs.
    trail: Vec<Crumb>,
    hostname: String,
    sections: Vec<SectionView>,
}

async fn load_device(state: &AppState, id: i64) -> Result<Device> {
    devices::get_device(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)
}

fn percent(t: Lang, value: f64) -> String {
    format_value(t, Unit::Percent, value)
}

/// Everything this machine can report, in sections (title message id, candidates).
async fn candidates(t: Lang, paths: &Paths) -> Vec<(&'static str, Vec<Candidate>)> {
    // The first channel's value now; CPU load needs two readings and shows none.
    let current = |metric: &LocalMetric| {
        let mut sensor = local::LocalSensor::new(metric.clone(), paths.clone());
        async move {
            sensor
                .read_once()
                .await
                .ok()
                .and_then(|r| r.into_iter().next())
                .and_then(|c| c.value.map(|v| format_value(t, c.unit, v)))
                .unwrap_or_else(|| "–".to_owned())
        }
    };
    let mut general = Vec::new();
    for (metric, name, detail) in [
        (LocalMetric::Cpu, "local-name-cpu", "local-detail-cpu"),
        (LocalMetric::Load, "local-name-load", "local-detail-load"),
        (
            LocalMetric::Memory,
            "local-name-memory",
            "local-detail-memory",
        ),
        (
            LocalMetric::Uptime,
            "local-name-uptime",
            "local-detail-uptime",
        ),
    ] {
        let value = current(&metric).await;
        general.push(Candidate {
            metric,
            name: t.get(name),
            detail: t.get(detail),
            value,
            preselect: true,
            limits: Vec::new(),
        });
    }

    let mut disks = Vec::new();
    for fs in local::filesystems(paths) {
        let network = matches!(fs.fstype.as_str(), "nfs" | "nfs4" | "cifs" | "smb3");
        let value = match local::disk_usage(&fs.mount).await {
            Ok(u) => t.get2(
                "local-disk-usage",
                "used",
                percent(t, u.used_pct),
                "size",
                format_value(t, Unit::Bytes, u.size),
            ),
            Err(_) => "–".to_owned(),
        };
        disks.push(Candidate {
            name: t.get1("local-name-disk", "mount", fs.mount.as_str()),
            detail: format!("{} · {}", fs.fstype, fs.source),
            value,
            preselect: !network,
            limits: Vec::new(),
            metric: LocalMetric::Disk { mount: fs.mount },
        });
    }

    // Inputs without a value right now (e.g. Wi-Fi switched off) are not offered.
    let temperatures = local::temperatures(paths)
        .into_iter()
        .filter(|temp| temp.celsius.is_some())
        .map(|temp| {
            let limits = temp.limits();
            let detail = [
                limits
                    .warn_above
                    .map(|v| t.get1("local-temp-max", "value", format_value(t, Unit::Celsius, v))),
                limits.down_above.map(|v| {
                    t.get1(
                        "local-temp-crit",
                        "value",
                        format_value(t, Unit::Celsius, v),
                    )
                }),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            Candidate {
                name: t.get1("local-name-temperature", "sensor", temp.key.as_str()),
                detail,
                value: temp
                    .celsius
                    .map_or_else(String::new, |c| format_value(t, Unit::Celsius, c)),
                preselect: false,
                limits: vec![("temperature", limits)],
                metric: LocalMetric::Temperature { sensor: temp.key },
            }
        })
        .collect();

    let network = local::interfaces(paths)
        .into_iter()
        .map(|i| Candidate {
            name: t.get1("local-name-traffic", "interface", i.name.as_str()),
            detail: if i.physical {
                t.get("local-interface-physical")
            } else {
                t.get("local-interface-virtual")
            },
            value: i.state.clone(),
            preselect: i.physical && i.state == "up",
            limits: Vec::new(),
            metric: LocalMetric::Traffic { interface: i.name },
        })
        .collect();

    vec![
        ("local-section-general", general),
        ("local-section-disks", disks),
        ("local-section-temperatures", temperatures),
        ("local-section-network", network),
    ]
}

/// What a device already measures locally.
async fn existing_targets(state: &AppState, device_id: i64) -> Result<HashSet<String>> {
    Ok(store::list_sensors(&state.pool, device_id)
        .await?
        .into_iter()
        .filter_map(|s| match s.spec {
            SensorSpec::Local(metric) => Some(metric.target()),
            _ => None,
        })
        .collect())
}

pub async fn page(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let device = load_device(&state, id).await?;
    let paths = Paths::default();
    let existing = existing_targets(&state, id).await?;
    let sections = candidates(user.lang, &paths)
        .await
        .into_iter()
        .filter(|(_, list)| !list.is_empty())
        .map(|(title, list)| SectionView {
            title: user.lang.get(title),
            candidates: list
                .into_iter()
                .map(|c| {
                    let target = c.metric.target();
                    CandidateView {
                        exists: existing.contains(&target),
                        selected: c.preselect,
                        target,
                        name: c.name,
                        detail: c.detail,
                        value: c.value,
                    }
                })
                .collect(),
        })
        .collect();
    let page = PageTemplate {
        layout: user.layout(),
        t: user.lang,
        trail: group_trail(&state.pool, device.group_id).await?,
        hostname: local::hostname(&paths),
        device,
        sections,
    };
    Ok(Html(page.render()?))
}

/// Creates the ticked values (fields `target=<target>`), with default limits.
pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(fields): Form<Vec<(String, String)>>,
) -> Result<Response> {
    user.require_admin()?;
    let csrf = fields
        .iter()
        .find(|(k, _)| k == "csrf_token")
        .map_or("", |(_, v)| v.as_str());
    user.verify_csrf(csrf)?;
    let device = load_device(&state, id).await?;
    if fields.len() > MAX_FIELDS {
        return Err(Error::invalid(Msg::new("form-too-large")));
    }
    let wanted: HashSet<&str> = fields
        .iter()
        .filter(|(k, _)| k == "target")
        .map(|(_, v)| v.as_str())
        .collect();
    if wanted.is_empty() {
        return Err(Error::invalid(Msg::new("local-select-value")));
    }
    let mut existing = existing_targets(&state, id).await?;
    let mut created = 0usize;
    // Only what this machine offers right now can be created.
    for (_, list) in candidates(user.lang, &Paths::default()).await {
        for c in list {
            let target = c.metric.target();
            if !wanted.contains(target.as_str()) || existing.contains(&target) {
                continue;
            }
            let spec = SensorSpec::Local(c.metric.clone());
            let sensor = create_named(&state, id, &c.name, &spec).await?;
            let mut limits = c.metric.default_limits();
            limits.extend(c.limits.iter().copied());
            set_limits(&state, sensor, &limits).await?;
            let name = store::get_sensor(&state.pool, sensor)
                .await?
                .map_or_else(|| c.name.clone(), |s| s.name);
            crate::web::sensors::audit_created(&state, &user, sensor, &name, &device).await;
            existing.insert(target);
            created += 1;
        }
    }
    tracing::info!(user = %user.username, device = id, created, "local sensors created");
    Ok(Redirect::to(&format!("/devices/{id}/sensors?msg=sensors_added")).into_response())
}

/// Creates the sensor; a name taken on this device gets ` #2`, ` #3` … appended.
pub(super) async fn create_named(
    state: &AppState,
    device: i64,
    name: &str,
    spec: &SensorSpec,
) -> Result<i64> {
    let base: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    let mut candidate = base.clone();
    for n in 2..=20 {
        match store::create_sensor(&state.pool, device, &candidate, spec).await {
            Err(Error::Conflict(_)) => {
                let suffix = format!(" #{n}");
                let short: String = base
                    .chars()
                    .take(MAX_NAME_CHARS - suffix.chars().count())
                    .collect();
                candidate = format!("{short}{suffix}");
            }
            other => return other,
        }
    }
    store::create_sensor(&state.pool, device, &candidate, spec).await
}

async fn set_limits(
    state: &AppState,
    sensor: i64,
    limits: &[(&'static str, Thresholds)],
) -> Result<()> {
    let limits: Vec<(&str, Thresholds)> = limits
        .iter()
        .copied()
        .filter(|(_, l)| *l != Thresholds::default())
        .collect();
    if limits.is_empty() {
        return Ok(());
    }
    let record = store::get_sensor(&state.pool, sensor)
        .await?
        .ok_or(Error::NotFound)?;
    let by_channel: Vec<(i64, Thresholds)> = limits
        .iter()
        .filter_map(|(key, l)| {
            record
                .channels
                .iter()
                .find(|c| c.key == *key)
                .map(|c| (c.id, *l))
        })
        .collect();
    store::set_thresholds(&state.pool, sensor, &by_channel).await
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;
    use crate::devices::{DeviceInput, Polling};
    use crate::storage::devices::save_device;
    use crate::web::testutil::*;

    async fn server_device(h: &Harness) -> i64 {
        save_device(
            &h.state.pool,
            None,
            DeviceInput {
                kind: DeviceKind::Server,
                name: "Server".to_owned(),
                host: "192.0.2.10".to_owned(),
                group_id: None,
                profile_id: None,
                polling: Polling::default(),
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn the_page_lists_this_machine_and_creates_sensors_with_limits() {
        let h = harness(true).await;
        let device = server_device(&h).await;
        let (admin, csrf) = login(&h, "anna").await;
        let path = format!("/devices/{device}/server");
        let page = send(&h.app, get(&path, Some(&admin))).await;
        assert_eq!(page.status, StatusCode::OK);
        for target in ["cpu", "load", "memory", "uptime", "disk:/"] {
            assert!(
                page.body.contains(&format!("value=\"{target}\"")),
                "{target}"
            );
        }

        let form = format!(
            "csrf_token={csrf}&target=cpu&target=memory&target=disk%3A%2F&target=disk%3A%2Fnot-mounted"
        );
        let done = send(&h.app, post(&path, Some(&admin), &form)).await;
        assert_eq!(done.status, StatusCode::SEE_OTHER);
        let sensors = store::list_sensors(&h.state.pool, device).await.unwrap();
        let mut targets: Vec<String> = sensors
            .iter()
            .filter_map(|s| match &s.spec {
                SensorSpec::Local(m) => Some(m.target()),
                _ => None,
            })
            .collect();
        targets.sort();
        assert_eq!(targets, ["cpu", "disk:/", "memory"], "only what is offered");
        let disk = sensors
            .iter()
            .find(|s| s.spec == SensorSpec::Local(LocalMetric::Disk { mount: "/".into() }))
            .unwrap();
        assert_eq!(disk.name, "Dateisystem /");
        assert_eq!(disk.channels[0].thresholds.warn_above, Some(85.0));
        assert_eq!(disk.channels[0].thresholds.down_above, Some(95.0));

        // A second time creates nothing new; the page marks them as present.
        send(&h.app, post(&path, Some(&admin), &form)).await;
        assert_eq!(
            store::list_sensors(&h.state.pool, device)
                .await
                .unwrap()
                .len(),
            3
        );
        let again = send(&h.app, get(&path, Some(&admin))).await;
        let cpu = again.body.split("value=\"cpu\"").nth(1).unwrap();
        assert!(cpu[..cpu.find('>').unwrap()].contains("disabled checked"));
        let sensors_page = send(
            &h.app,
            get(&format!("/devices/{device}/sensors"), Some(&admin)),
        )
        .await;
        assert!(sensors_page.body.contains(&format!("href=\"{path}\"")));
        assert!(sensors_page.body.contains("<option value=\"server\">"));
        let disk_page = send(&h.app, get(&format!("/sensors/{}", disk.id), Some(&admin))).await;
        assert!(disk_page.body.contains("Dieser Server: Dateisystem /"));
    }

    #[tokio::test]
    async fn only_admins_with_a_valid_token_may_use_it() {
        let h = harness(true).await;
        let device = server_device(&h).await;
        let path = format!("/devices/{device}/server");
        let (viewer, vcsrf) = login(&h, "bert").await;
        assert_eq!(
            send(&h.app, get(&path, Some(&viewer))).await.status,
            StatusCode::FORBIDDEN
        );
        let form = format!("csrf_token={vcsrf}&target=cpu");
        assert_eq!(
            send(&h.app, post(&path, Some(&viewer), &form)).await.status,
            StatusCode::FORBIDDEN
        );
        let (admin, csrf) = login(&h, "anna").await;
        let bad = send(
            &h.app,
            post(&path, Some(&admin), "csrf_token=wrong&target=cpu"),
        )
        .await;
        assert_eq!(bad.status, StatusCode::FORBIDDEN);
        let none = send(
            &h.app,
            post(&path, Some(&admin), &format!("csrf_token={csrf}")),
        )
        .await;
        assert_eq!(none.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            store::list_sensors(&h.state.pool, device)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn every_device_reaches_the_page_from_the_add_sensor_form() {
        let h = harness(true).await;
        let device = save_device(
            &h.state.pool,
            None,
            DeviceInput {
                kind: DeviceKind::Switch,
                name: "Switch".to_owned(),
                host: "192.0.2.20".to_owned(),
                group_id: None,
                profile_id: None,
                polling: Polling::default(),
            },
        )
        .await
        .unwrap();
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(
            &h.app,
            get(&format!("/devices/{device}/sensors"), Some(&admin)),
        )
        .await;
        assert!(page.body.contains("<option value=\"server\">"));
        assert!(
            !page
                .body
                .contains(&format!("href=\"/devices/{device}/server\"")),
            "no button for a switch"
        );
        let reply = send(
            &h.app,
            post(
                &format!("/devices/{device}/sensors"),
                Some(&admin),
                &format!("csrf_token={csrf}&kind=server&name="),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        assert_eq!(reply.location(), format!("/devices/{device}/server"));
        assert!(
            store::list_sensors(&h.state.pool, device)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn the_page_is_offered_for_servers_and_this_machine() {
        let device = |host: &str, kind| Device {
            id: 1,
            name: "x".into(),
            host: host.into(),
            group_id: None,
            profile_id: None,
            profile_name: None,
            polling: Polling::default(),
            kind,
        };
        assert!(offered(&device("192.0.2.1", DeviceKind::Server), false));
        assert!(offered(&device("127.0.0.1", DeviceKind::Generic), false));
        assert!(offered(&device("LocalHost", DeviceKind::Generic), false));
        assert!(offered(&device("192.0.2.1", DeviceKind::Switch), true));
        assert!(!offered(&device("192.0.2.1", DeviceKind::Switch), false));
    }
}
