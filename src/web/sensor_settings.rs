//! The settings page of a sensor (`/sensors/<id>/edit`, administrators only): name, polling
//! interval, what the sensor checks (custom OID and service sensors), the limits of its channels
//! and the ratings of named status values, in one form with one "Save".
//!
//! A section is only changed when its fields are sent (the page always sends all of them), and the
//! whole form is checked before anything is written: an error changes nothing and shows the page
//! again with the input.

use std::collections::HashMap;

use askama::Template;
use axum::Form;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};

use crate::audit::{Action, Changes, Kind};
use crate::devices::Device;
use crate::error::{Error, Result};
use crate::i18n::{Lang, Msg};
use crate::sensors::Unit;
use crate::sensors::format::{channel_label, input_unit};
use crate::sensors::snmp::OidMode;
use crate::sensors::spec::SensorSpec;
use crate::status::{Status, Thresholds, ValueState};
use crate::storage::sensors::{self as store, SensorRecord};
use crate::web::audit::{self, Object};
use crate::web::auth::CurrentUser;
use crate::web::sensor_detail::{
    StatesView, fields_of, load, named_values, parse_limits, rated_channel, states_changes,
    states_view,
};
use crate::web::sensor_text::ValueFormat;
use crate::web::sensors::{SensorForm, SensorFormValues, has_settings};
use crate::web::{AppState, Crumb, Layout, group_trail};

/// The limit inputs of one channel.
struct LimitsView {
    id: i64,
    label: String,
    unit_label: String,
    /// Warn above, warn below, down above, down below (in `input_unit`).
    fields: [String; 4],
}

#[derive(Template)]
#[template(path = "sensor_settings.html")]
struct SettingsTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    device: Device,
    /// Groups above the device, for the breadcrumbs.
    trail: Vec<Crumb>,
    sensor_id: i64,
    /// The stored name (heading, breadcrumbs); the field may hold a rejected one.
    sensor_name: String,
    name: String,
    /// The own interval, empty for the device's, or the rejected input.
    interval: String,
    own_interval: bool,
    /// What the sensor checks (custom OID, service sensors); `None` for the other types.
    form: Option<SensorFormValues>,
    limits: Vec<LimitsView>,
    states: Option<StatesView>,
    error: Option<String>,
}

impl SettingsTemplate {
    fn tls_hint(&self) -> String {
        self.t.get2(
            "service-tls-hint",
            "warn",
            crate::sensors::spec::TLS_WARN_DAYS as u32,
            "down",
            crate::sensors::spec::TLS_DOWN_DAYS as u32,
        )
    }
}

async fn render(
    state: &AppState,
    user: &CurrentUser,
    sensor: &SensorRecord,
    device: Device,
    submitted: Option<&HashMap<String, String>>,
    error: Option<String>,
) -> Result<String> {
    let t = user.lang;
    let sent = |name: &str| submitted.and_then(|f| f.get(name)).cloned();
    let value_format = ValueFormat::for_sensor(&sensor.spec, &state.mib_index());
    let limits = sensor
        .channels
        .iter()
        .filter(|c| c.unit != Unit::State)
        .map(|c| {
            let stored = fields_of(t, &c.thresholds, c.unit);
            let mut fields = stored.clone();
            for (field, prefix) in fields.iter_mut().zip(["wa", "wb", "da", "db"]) {
                if let Some(text) = sent(&format!("{prefix}_{}", c.id)) {
                    *field = text;
                }
            }
            LimitsView {
                id: c.id,
                label: t.msg(&channel_label(&c.key)),
                unit_label: match (&value_format.unit_label, c.unit) {
                    (Some(label), Unit::PerSecond) => format!("{label}/s"),
                    (Some(label), Unit::Count) => label.clone(),
                    _ => input_unit(c.unit).to_owned(),
                },
                fields,
            }
        })
        .collect();
    let mut states = states_view(sensor, &value_format);
    if let (Some(st), Some(form)) = (&mut states, submitted) {
        for row in &mut st.rows {
            if let Some(status) = form
                .get(&format!("st_{}", row.value))
                .and_then(|s| Status::parse(s))
            {
                row.status = status.as_str();
            }
        }
    }
    let form = if has_settings(&sensor.spec) {
        match submitted {
            Some(fields) => Some(SensorFormValues {
                editing: true,
                ..SensorForm::from_fields(fields).values()
            }),
            None => SensorFormValues::from_spec(&sensor.spec),
        }
        .map(|mut f| {
            f.kind = sensor.spec.kind().to_owned();
            f
        })
    } else {
        None
    };
    let page = SettingsTemplate {
        t,
        layout: user.layout(),
        trail: group_trail(&state.pool, device.group_id).await?,
        device,
        sensor_id: sensor.id,
        sensor_name: sensor.name.clone(),
        name: sent("name").unwrap_or_else(|| sensor.name.clone()),
        interval: sent("interval").unwrap_or_else(|| {
            sensor
                .interval_secs
                .map_or_else(String::new, |s| s.to_string())
        }),
        own_interval: sensor.interval_secs.is_some(),
        form,
        limits,
        states,
        error,
    };
    Ok(page.render()?)
}

pub async fn edit_form(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let (sensor, device) = load(&state, id).await?;
    Ok(Html(
        render(&state, &user, &sensor, device, None, None).await?,
    ))
}

/// What a submitted form changes, checked but not written yet.
struct Plan {
    name: Option<String>,
    interval: Option<Option<u32>>,
    spec: Option<SensorSpec>,
    limits: Vec<(i64, Thresholds)>,
    states: Option<(i64, Vec<ValueState>)>,
}

fn plan(
    state: &AppState,
    user: &CurrentUser,
    sensor: &SensorRecord,
    device: &Device,
    fields: &HashMap<String, String>,
) -> Result<Plan> {
    let name = match fields.get("name") {
        Some(name) => {
            let name = crate::devices::validate_name(name).map_err(Error::invalid)?;
            (name != sensor.name).then(|| name.to_owned())
        }
        None => None,
    };
    let interval = match fields.get("interval").map(|s| s.trim()) {
        None => None,
        Some("") => Some(None),
        Some(text) => {
            let secs = text
                .parse::<u32>()
                .map_err(|_| Error::invalid(Msg::new("detail-interval-invalid")))?;
            crate::devices::Polling {
                interval_secs: secs,
                ..device.polling
            }
            .validate()
            .map_err(Error::invalid)?;
            Some(Some(secs))
        }
    }
    .filter(|secs| *secs != sensor.interval_secs);
    let spec = if has_settings(&sensor.spec) && fields.contains_key("kind") {
        let mut form = SensorForm::from_fields(fields);
        // The type cannot change; the name is the one above.
        form.kind = sensor.spec.kind().to_owned();
        form.name.clone_from(&sensor.name);
        let (_, spec) = form.to_spec(user.lang, &state.mib_index(), &device.host)?;
        spec.validate().map_err(Error::invalid)?;
        (spec != sensor.spec).then_some(spec)
    } else {
        None
    };
    // Limits of the channels whose fields were sent.
    let sent: Vec<_> = sensor
        .channels
        .iter()
        .filter(|c| {
            ["wa", "wb", "da", "db"]
                .iter()
                .any(|p| fields.contains_key(&format!("{p}_{}", c.id)))
        })
        .cloned()
        .collect();
    let limits = parse_limits(&sent, fields)?
        .into_iter()
        .filter(|(id, new)| {
            sensor
                .channels
                .iter()
                .any(|c| c.id == *id && c.thresholds != *new)
        })
        .collect();
    Ok(Plan {
        name,
        interval,
        spec,
        limits,
        states: planned_states(state, sensor, fields)?,
    })
}

/// New ratings of the named status values, if they were sent and differ from what the page showed
/// (saving the page unchanged does not turn suggestions into stored ratings).
fn planned_states(
    state: &AppState,
    sensor: &SensorRecord,
    fields: &HashMap<String, String>,
) -> Result<Option<(i64, Vec<ValueState>)>> {
    let Some(channel) = rated_channel(sensor) else {
        return Ok(None);
    };
    let format = ValueFormat::for_sensor(&sensor.spec, &state.mib_index());
    let values = named_values(channel, &format);
    if !values
        .iter()
        .any(|(v, _)| fields.contains_key(&format!("st_{v}")))
    {
        return Ok(None);
    }
    let shown = states_view(sensor, &format)
        .map(|v| v.rows)
        .unwrap_or_default();
    let mut changed = false;
    let mut states = Vec::new();
    for (value, label) in values {
        let status = match fields
            .get(&format!("st_{value}"))
            .and_then(|s| Status::parse(s))
        {
            Some(s @ (Status::Up | Status::Warning | Status::Down)) => s,
            _ => {
                return Err(Error::invalid(
                    Msg::new("states-choose").with("value", label.as_str()),
                ));
            }
        };
        changed |= shown
            .iter()
            .find(|r| r.value == value)
            .is_none_or(|r| r.status != status.as_str());
        states.push(ValueState {
            value,
            label,
            status,
        });
    }
    Ok(changed.then_some((channel.id, states)))
}

/// Saves the settings page; see the module documentation.
pub async fn save(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(fields): Form<HashMap<String, String>>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(fields.get("csrf_token").map_or("", String::as_str))?;
    let (sensor, device) = load(&state, id).await?;
    let outcome = match plan(&state, &user, &sensor, &device, &fields) {
        // The name first: a name already taken stops before anything else is written.
        Ok(plan) => match &plan.name {
            Some(name) => store::rename_sensor(&state.pool, id, name)
                .await
                .map(|()| plan),
            None => Ok(plan),
        },
        Err(e) => Err(e),
    };
    let plan = match outcome {
        Ok(plan) => plan,
        Err(Error::Invalid(message) | Error::Conflict(message)) => {
            let html = render(
                &state,
                &user,
                &sensor,
                device,
                Some(&fields),
                Some(user.lang.msg(&message)),
            )
            .await?;
            return Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response());
        }
        Err(e) => return Err(e),
    };
    let mut changes = Changes::new();
    if let Some(name) = &plan.name {
        changes = changes.field("audit-field-name", &sensor.name, name);
    }
    if let Some(spec) = &plan.spec {
        store::update_spec(&state.pool, id, spec).await?;
        changes = changes.merge(settings_changes(&sensor.spec, spec));
    }
    if let Some(secs) = plan.interval {
        store::set_interval(&state.pool, id, secs, &device.polling).await?;
        let shown = |s: Option<u32>| s.map_or_else(String::new, |s| s.to_string());
        changes = changes.field(
            "audit-field-interval",
            shown(sensor.interval_secs),
            shown(secs),
        );
    }
    if !plan.limits.is_empty() {
        store::set_thresholds(&state.pool, id, &plan.limits).await?;
        for (channel, new) in &plan.limits {
            if let Some(c) = sensor.channels.iter().find(|c| c.id == *channel) {
                changes = changes.limits(channel_label(&c.key), c.unit, &c.thresholds, new);
            }
        }
    }
    if let Some((channel, states)) = &plan.states {
        store::set_value_states(&state.pool, id, *channel, states).await?;
        let old = sensor
            .channels
            .iter()
            .find(|c| c.id == *channel)
            .map(|c| c.states.clone())
            .unwrap_or_default();
        changes = changes.merge(states_changes(&old, states));
    }
    tracing::info!(user = %user.username, sensor = id, "sensor settings saved");
    let name = plan.name.as_deref().unwrap_or(&sensor.name);
    audit::record(
        &state,
        &user,
        Action::Changed,
        Object::new(Kind::Sensor, id, name),
        changes,
    )
    .await;
    Ok(Redirect::to(&format!("/sensors/{id}?msg=sensor_settings_saved")).into_response())
}

/// The changed settings for the audit log, field by field (labels of the form).
fn settings_changes(old: &SensorSpec, new: &SensorSpec) -> Changes {
    let yes_no = |b: bool| if b { "audit-yes" } else { "audit-no" };
    let text = |o: &Option<String>| o.clone().unwrap_or_default();
    let changes = Changes::new();
    match (old, new) {
        (
            SensorSpec::Oid {
                oid: o_oid,
                mode: o_mode,
                scale: o_scale,
                unit: o_unit,
            },
            SensorSpec::Oid {
                oid,
                mode,
                scale,
                unit,
            },
        ) => {
            let mode_id = |m: &OidMode| match m {
                OidMode::Gauge => "sensors-mode-gauge",
                OidMode::Counter => "sensors-mode-counter",
            };
            changes
                .field(
                    "sensors-oid-only",
                    crate::sensors::spec::format_oid(o_oid),
                    crate::sensors::spec::format_oid(oid),
                )
                .label("sensors-mode", mode_id(o_mode), mode_id(mode))
                .field("sensors-scale", o_scale, scale)
                .field("sensors-unit", text(o_unit), text(unit))
        }
        (SensorSpec::Http(o), SensorSpec::Http(n)) => changes
            .field("service-url", &o.url, &n.url)
            .field("service-expect", &o.expect, &n.expect)
            .field("service-keyword", text(&o.keyword), text(&n.keyword))
            .label(
                "service-follow-redirects",
                yes_no(o.follow_redirects),
                yes_no(n.follow_redirects),
            )
            .label(
                "service-verify-tls",
                yes_no(o.verify_tls),
                yes_no(n.verify_tls),
            )
            .label("service-head", yes_no(o.head), yes_no(n.head)),
        (SensorSpec::Tcp(o), SensorSpec::Tcp(n)) => changes.field("service-port", o.port, n.port),
        (SensorSpec::Dns(o), SensorSpec::Dns(n)) => {
            let ip = |a: &Option<std::net::IpAddr>| a.map(|a| a.to_string()).unwrap_or_default();
            changes
                .field("service-dns-name", &o.name, &n.name)
                .field("service-dns-type", o.record.label(), n.record.label())
                .field("service-port", o.port, n.port)
                .field("service-dns-expect", ip(&o.expect), ip(&n.expect))
        }
        (SensorSpec::Tls(o), SensorSpec::Tls(n)) => changes
            .field("service-port", o.port, n.port)
            .field(
                "service-server-name",
                text(&o.server_name),
                text(&n.server_name),
            )
            .label("service-verify-tls", yes_no(o.verify), yes_no(n.verify)),
        _ => changes,
    }
}
