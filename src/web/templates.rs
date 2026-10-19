//! Device template pages (admin only): list, details, "save as template" on a device and
//! "apply template" (also used right after creating a device).

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use askama::Template;
use axum::Form;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::devices::Device;
use crate::discovery::{DiscoveredInterface, discover_interfaces};
use crate::error::{Error, Result};
use crate::i18n::{Lang, Msg};
use crate::sensors::spec::SensorSpec;
use crate::snmp::{SnmpClient, SnmpError, SnmpTarget};
use crate::storage::templates::{self as store, TemplateRecord};
use crate::storage::{devices, sensors};
use crate::templates::{
    self as tpl, ApplyError, BasicKind, BuildOptions, PortChoice, PortRule, PortSelection,
};
use crate::web::auth::{CsrfForm, CurrentUser};
use crate::web::{AppState, Layout, flash_message};

/// Template work never blocks a request for long, whatever the device is configured to.
const MAX_TIMEOUT_SECS: u32 = 10;
const MAX_RETRIES: u32 = 1;
const MAX_FORM_FIELDS: usize = 5_000;

/// An SNMP session to a device with capped timeouts. The outer error is a storage problem, the
/// inner one means the device could not be reached.
async fn connect(
    state: &AppState,
    device: &Device,
) -> Result<std::result::Result<SnmpClient, SnmpError>> {
    let creds = crate::web::snmp_credentials(state, device).await?;
    let target = SnmpTarget {
        host: device.host.clone(),
        port: device.polling.port,
        timeout: Duration::from_secs(u64::from(device.polling.timeout_secs.min(MAX_TIMEOUT_SECS))),
        retries: device.polling.retries.min(MAX_RETRIES),
    };
    Ok(SnmpClient::connect(&target, &creds).await)
}

async fn load_device(state: &AppState, id: i64) -> Result<Device> {
    devices::get_device(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)
}

async fn load_template(state: &AppState, id: i64) -> Result<TemplateRecord> {
    store::get_template(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)
}

// ---------------------------------------------------------------------------------------------
// List and details

struct ListRow {
    id: i64,
    name: String,
    summary: String,
    object_id: String,
    source: String,
    devices: i64,
}

#[derive(Template)]
#[template(path = "templates_list.html")]
struct ListTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    rows: Vec<ListRow>,
    flash: Option<String>,
}

#[derive(Deserialize)]
pub struct FlashQuery {
    msg: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<FlashQuery>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let usage: HashMap<i64, i64> = store::usage(&state.pool).await?.into_iter().collect();
    let rows = store::list_templates(&state.pool)
        .await?
        .into_iter()
        .map(|t| ListRow {
            summary: t.body.summary(user.lang),
            object_id: t.object_id.unwrap_or_default(),
            devices: usage.get(&t.id).copied().unwrap_or(0),
            id: t.id,
            name: t.name,
            source: t.source,
        })
        .collect();
    let page = ListTemplate {
        layout: user.layout(),
        t: user.lang,
        rows,
        flash: flash_message(user.lang, query.msg.as_deref()),
    };
    Ok(Html(page.render()?))
}

/// One line of the template's contents.
struct ItemView {
    name: String,
    detail: String,
    state: String,
}

fn state_label(t: Lang, enabled: bool) -> String {
    t.get(if enabled { "active" } else { "inactive" })
}

fn limits_note(t: Lang, limits: &[tpl::Limits]) -> String {
    let states = limits.iter().any(|l| !l.states.is_empty());
    let thresholds = limits.iter().any(|l| !l.thresholds().is_empty());
    let id = match (thresholds, states) {
        (true, true) => "template-with-thresholds-and-states",
        (true, false) => "template-with-thresholds",
        (false, true) => "template-with-states",
        (false, false) => return String::new(),
    };
    format!(" · {}", t.get(id))
}

fn rule_items(t: Lang, what: &str, rule: &PortRule) -> Vec<ItemView> {
    match rule.selection {
        PortSelection::Same => rule
            .ports
            .iter()
            .map(|p| ItemView {
                name: p.name.clone(),
                detail: format!(
                    "{}{}",
                    t.get2(
                        "template-rule-port",
                        "what",
                        what,
                        "interface",
                        p.interface.as_str()
                    ),
                    limits_note(t, &p.limits)
                ),
                state: state_label(t, p.enabled),
            })
            .collect(),
        PortSelection::Up | PortSelection::All => vec![ItemView {
            name: format!("{what} {}", tpl::selection_label(t, rule)),
            detail: format!(
                "{}{}",
                t.get1("template-rule-each-port", "what", what),
                limits_note(t, &rule.limits)
            ),
            state: state_label(t, rule.enabled),
        }],
    }
}

fn items_of(t: Lang, body: &tpl::Body) -> Vec<ItemView> {
    let mut items: Vec<ItemView> = body
        .basics
        .iter()
        .map(|b| ItemView {
            name: b.name.clone(),
            detail: format!(
                "{}{}",
                match b.kind {
                    BasicKind::Ping => "Ping",
                    BasicKind::Uptime => "Uptime",
                },
                limits_note(t, &b.limits)
            ),
            state: state_label(t, b.enabled),
        })
        .collect();
    if let Some(rule) = &body.traffic {
        items.extend(rule_items(t, "Traffic", rule));
    }
    if let Some(rule) = &body.status {
        items.extend(rule_items(t, "Status", rule));
    }
    items.extend(body.oids.iter().map(|o| ItemView {
        name: o.name.clone(),
        detail: format!(
            "OID {} · {}{}{}{}",
            o.oid,
            t.get(if o.counter {
                "template-oid-counter"
            } else {
                "template-oid-value"
            }),
            if (o.scale - 1.0).abs() > f64::EPSILON {
                format!(", {}", t.get1("oid-detail-factor", "factor", o.scale))
            } else {
                String::new()
            },
            o.unit.as_deref().map_or_else(String::new, |u| format!(
                ", {}",
                t.get1("oid-detail-unit", "unit", u)
            )),
            limits_note(t, &o.limits)
        ),
        state: state_label(t, o.enabled),
    }));
    items
}

#[derive(Template)]
#[template(path = "template_detail.html")]
struct DetailTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    template: TemplateRecord,
    items: Vec<ItemView>,
    name: String,
    object_id: String,
    error: Option<String>,
    flash: Option<String>,
}

fn detail_page(
    user: &CurrentUser,
    template: TemplateRecord,
    error: Option<String>,
    flash: Option<String>,
    input: Option<(String, String)>,
) -> Result<String> {
    let (name, object_id) = input.unwrap_or_else(|| {
        (
            template.name.clone(),
            template.object_id.clone().unwrap_or_default(),
        )
    });
    let page = DetailTemplate {
        layout: user.layout(),
        t: user.lang,
        items: items_of(user.lang, &template.body),
        template,
        name,
        object_id,
        error,
        flash,
    };
    Ok(page.render()?)
}

pub async fn detail(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<FlashQuery>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let template = load_template(&state, id).await?;
    Ok(Html(detail_page(
        &user,
        template,
        None,
        flash_message(user.lang, query.msg.as_deref()),
        None,
    )?))
}

#[derive(Deserialize)]
pub struct EditForm {
    csrf_token: String,
    name: String,
    #[serde(default)]
    object_id: String,
}

pub async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<EditForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let template = load_template(&state, id).await?;
    match store::update_template(&state.pool, id, &form.name, &form.object_id).await {
        Ok(()) => {
            tracing::info!(user = %user.username, template = id, "device template updated");
            Ok(Redirect::to(&format!("/templates/{id}?msg=template_saved")).into_response())
        }
        Err(Error::Invalid(message) | Error::Conflict(message)) => {
            let html = detail_page(
                &user,
                template,
                Some(user.lang.msg(&message)),
                None,
                Some((form.name, form.object_id)),
            )?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response())
        }
        Err(e) => Err(e),
    }
}

pub async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    if !store::delete_template(&state.pool, id).await? {
        return Err(Error::NotFound);
    }
    tracing::info!(user = %user.username, template = id, "device template deleted");
    Ok(Redirect::to("/templates?msg=template_deleted").into_response())
}

// ---------------------------------------------------------------------------------------------
// "Als Vorlage speichern"

struct SourceSensor {
    id: i64,
    name: String,
    description: String,
    state: String,
}

struct PortGroup {
    field: &'static str,
    title: String,
    /// Interface names of the device's sensors of this kind (empty: none).
    current: Vec<String>,
    choice: &'static str,
}

#[derive(Template)]
#[template(path = "template_new.html")]
struct NewTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    device: Device,
    name: String,
    object_id: String,
    sensors: Vec<SourceSensor>,
    included: HashSet<i64>,
    ports: Vec<PortGroup>,
    /// Why the device could not be read (interfaces/identification), if so.
    notice: Option<String>,
    interfaces_known: bool,
    error: Option<String>,
}

impl NewTemplate {
    fn is_included(&self, id: &i64) -> bool {
        self.included.contains(id)
    }
}

/// What the device itself tells: its identification and interfaces (both optional).
struct DeviceFacts {
    object_id: Option<String>,
    interfaces: Option<Vec<DiscoveredInterface>>,
    failure: Option<String>,
}

async fn read_facts(
    state: &AppState,
    t: Lang,
    device: &Device,
    interfaces: bool,
) -> Result<DeviceFacts> {
    let failed = |object_id, e: SnmpError| DeviceFacts {
        object_id,
        interfaces: None,
        failure: Some(t.msg(&e.user_message())),
    };
    let mut client = match connect(state, device).await? {
        Ok(client) => client,
        Err(e) => return Ok(failed(None, e)),
    };
    let object_id = match tpl::read_object_id(&mut client).await {
        Ok(found) => found,
        Err(e) => return Ok(failed(None, e)),
    };
    let interfaces = if interfaces {
        match discover_interfaces(&mut client).await {
            Ok(found) => Some(found),
            Err(e) => return Ok(failed(object_id, e)),
        }
    } else {
        None
    };
    Ok(DeviceFacts {
        object_id,
        interfaces,
        failure: None,
    })
}

fn interface_index(spec: &SensorSpec, traffic: bool) -> Option<u32> {
    match (spec, traffic) {
        (SensorSpec::InterfaceTraffic { if_index }, true)
        | (SensorSpec::InterfaceStatus { if_index }, false) => Some(*if_index),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
async fn render_new(
    state: &AppState,
    user: &CurrentUser,
    device: Device,
    facts: &DeviceFacts,
    name: String,
    object_id: String,
    choices: Option<(HashSet<i64>, &'static str, &'static str)>,
    error: Option<String>,
) -> Result<String> {
    let all = sensors::list_sensors(&state.pool, device.id).await?;
    let mibs = state.mib_index();
    let interfaces = facts.interfaces.as_deref();
    let t = user.lang;
    let ports = [
        (true, "traffic", t.get("template-traffic")),
        (false, "status", t.get("template-interface-status")),
    ]
    .into_iter()
    .map(|(traffic, field, title)| {
        let mut current: Vec<String> = all
            .iter()
            .filter_map(|s| interface_index(&s.spec, traffic))
            .map(|index| {
                interfaces
                    .and_then(|list| list.iter().find(|i| i.index == index))
                    .map_or_else(|| format!("Index {index}"), |i| i.name.clone())
            })
            .collect();
        current.sort_by(|a, b| crate::ordering::natural_cmp(a, b));
        let default = if current.is_empty() { "skip" } else { "up" };
        let choice = match &choices {
            Some((_, traffic_choice, status_choice)) => {
                if traffic {
                    *traffic_choice
                } else {
                    *status_choice
                }
            }
            None => default,
        };
        PortGroup {
            field,
            title,
            current,
            choice,
        }
    })
    .collect();
    let others: Vec<SourceSensor> = all
        .iter()
        .filter(|s| {
            matches!(
                s.spec,
                SensorSpec::Ping | SensorSpec::Uptime | SensorSpec::Oid { .. }
            )
        })
        .map(|s| SourceSensor {
            id: s.id,
            name: s.name.clone(),
            description: crate::web::sensor_text::describe(t, &s.spec, &mibs),
            state: state_label(t, s.enabled),
        })
        .collect();
    let included = match choices {
        Some((included, _, _)) => included,
        None => others.iter().map(|s| s.id).collect(),
    };
    let page = NewTemplate {
        layout: user.layout(),
        t: user.lang,
        name,
        object_id,
        sensors: others,
        included,
        ports,
        notice: facts.failure.clone(),
        interfaces_known: facts.interfaces.is_some(),
        error,
        device,
    };
    Ok(page.render()?)
}

pub async fn new_form(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let device = load_device(&state, id).await?;
    let facts = read_facts(&state, user.lang, &device, true).await?;
    let name = device.name.clone();
    let object_id = facts.object_id.clone().unwrap_or_default();
    Ok(Html(
        render_new(&state, &user, device, &facts, name, object_id, None, None).await?,
    ))
}

fn choice_value(text: &str) -> &'static str {
    match text {
        "same" => "same",
        "up" => "up",
        "all" => "all",
        _ => "skip",
    }
}

pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(fields): Form<HashMap<String, String>>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(fields.get("csrf_token").map_or("", String::as_str))?;
    if fields.len() > MAX_FORM_FIELDS {
        return Err(Error::invalid(Msg::new("form-too-large")));
    }
    let device = load_device(&state, id).await?;
    let get = |key: &str| fields.get(key).map_or("", String::as_str);
    let (traffic, status) = (choice_value(get("traffic")), choice_value(get("status")));
    let include: HashSet<i64> = fields
        .iter()
        .filter(|(k, v)| k.starts_with("i_") && v.as_str() == "on")
        .filter_map(|(k, _)| k[2..].parse().ok())
        .collect();
    let needs_interfaces = traffic == "same" || status == "same";
    let facts = if needs_interfaces {
        read_facts(&state, user.lang, &device, true).await?
    } else {
        DeviceFacts {
            object_id: None,
            interfaces: None,
            failure: None,
        }
    };
    let all = sensors::list_sensors(&state.pool, id).await?;
    let built = tpl::build(
        &all,
        facts.interfaces.as_deref(),
        &BuildOptions {
            include: include.clone(),
            traffic: PortChoice::parse(traffic),
            status: PortChoice::parse(status),
        },
    );
    let outcome = match built {
        Ok(body) => {
            store::create_template(
                &state.pool,
                get("name"),
                Some(get("object_id")),
                &device.name,
                &body,
            )
            .await
        }
        Err(message) => Err(Error::Invalid(message)),
    };
    match outcome {
        Ok(template) => {
            store::set_template_of_device(&state.pool, id, Some(template)).await?;
            tracing::info!(user = %user.username, device = id, template, "device template created");
            Ok(
                Redirect::to(&format!("/templates/{template}?msg=template_created"))
                    .into_response(),
            )
        }
        Err(Error::Invalid(message) | Error::Conflict(message)) => {
            let html = render_new(
                &state,
                &user,
                device,
                &facts,
                get("name").to_owned(),
                get("object_id").to_owned(),
                Some((include, traffic, status)),
                Some(user.lang.msg(&message)),
            )
            .await?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response())
        }
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------------------------
// "Vorlage anwenden"

pub struct TemplateOption {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

#[derive(Template)]
#[template(path = "template_apply.html")]
struct ApplyTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    device: Device,
    options: Vec<TemplateOption>,
    object_id: Option<String>,
    detected: Option<String>,
    notice: Option<String>,
}

pub async fn apply_form(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let device = load_device(&state, id).await?;
    let templates = store::list_templates(&state.pool).await?;
    let facts = read_facts(&state, user.lang, &device, false).await?;
    let detected = facts
        .object_id
        .as_deref()
        .and_then(|oid| tpl::detect(&templates, oid, |t| t.object_id.as_deref()));
    let current = store::template_of_device(&state.pool, id).await?;
    let preselect = detected.map(|t| t.id).or(current);
    let options = templates
        .iter()
        .map(|t| TemplateOption {
            value: t.id.to_string(),
            label: format!("{} ({})", t.name, t.body.summary(user.lang)),
            selected: Some(t.id) == preselect,
        })
        .collect();
    let page = ApplyTemplate {
        layout: user.layout(),
        t: user.lang,
        options,
        object_id: facts.object_id.clone(),
        detected: detected.map(|t| t.name.clone()),
        notice: facts.failure,
        device,
    };
    Ok(Html(page.render()?))
}

#[derive(Template)]
#[template(path = "template_result.html")]
struct ResultTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    device: Device,
    heading: String,
    template_name: Option<String>,
    created: Vec<String>,
    present: usize,
    skipped: Vec<String>,
    failure: Option<String>,
}

impl ResultTemplate {
    /// "3 Sensoren angelegt, 2 schon vorhanden, 1 übersprungen."
    fn summary(&self) -> String {
        let t = self.t;
        let mut text = t.get1("template-result-created", "count", self.created.len());
        if self.present > 0 {
            text.push_str(&t.get1("template-result-present", "count", self.present));
        }
        if !self.skipped.is_empty() {
            text.push_str(&t.get1("template-result-skipped", "count", self.skipped.len()));
        }
        text.push('.');
        text
    }
}

/// Which template to apply.
pub enum Choice {
    /// The one whose identification matches the device's `sysObjectID`.
    Detect,
    Id(i64),
}

/// Applies a template to a device and renders the result page (also after creating a device).
pub async fn apply_and_render(
    state: &AppState,
    user: &CurrentUser,
    device: Device,
    choice: Choice,
    heading: String,
) -> Result<String> {
    let mut page = ResultTemplate {
        layout: user.layout(),
        t: user.lang,
        heading,
        template_name: None,
        created: Vec::new(),
        present: 0,
        skipped: Vec::new(),
        failure: None,
        device,
    };
    let mut client = match connect(state, &page.device).await? {
        Ok(client) => client,
        Err(e) => {
            page.failure = Some(user.lang.msg(&e.user_message()));
            return Ok(page.render()?);
        }
    };
    let template = match choice {
        Choice::Id(id) => load_template(state, id).await?,
        Choice::Detect => {
            let templates = store::list_templates(&state.pool).await?;
            let object_id = match tpl::read_object_id(&mut client).await {
                Ok(id) => id,
                Err(e) => {
                    page.failure = Some(user.lang.msg(&e.user_message()));
                    return Ok(page.render()?);
                }
            };
            let found = object_id
                .as_deref()
                .and_then(|oid| tpl::detect(&templates, oid, |t| t.object_id.as_deref()).cloned());
            match found {
                Some(t) => t,
                None => {
                    page.failure = Some(match object_id.as_deref() {
                        Some(oid) => user.lang.get1("template-no-match", "oid", oid),
                        None => user.lang.get("template-no-object-id"),
                    });
                    return Ok(page.render()?);
                }
            }
        }
    };
    page.template_name = Some(template.name.clone());
    match tpl::apply(&state.pool, &mut client, page.device.id, &template.body).await {
        Ok(outcome) => {
            store::set_template_of_device(&state.pool, page.device.id, Some(template.id)).await?;
            tracing::info!(
                user = %user.username,
                device = page.device.id,
                template = template.id,
                created = outcome.created.len(),
                skipped = outcome.skipped.len(),
                "device template applied"
            );
            page.created = outcome.created;
            page.present = outcome.present;
            page.skipped = outcome.skipped.iter().map(|m| user.lang.msg(m)).collect();
        }
        Err(ApplyError::Device(e)) => page.failure = Some(user.lang.msg(&e.user_message())),
        Err(ApplyError::Store(e)) => return Err(e),
    }
    Ok(page.render()?)
}

#[derive(Deserialize)]
pub struct ApplyForm {
    csrf_token: String,
    template_id: String,
}

pub async fn apply(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<ApplyForm>,
) -> Result<Html<String>> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let device = load_device(&state, id).await?;
    let choice = match form.template_id.trim() {
        "auto" => Choice::Detect,
        text => Choice::Id(
            text.parse()
                .map_err(|_| Error::invalid(Msg::new("template-choose")))?,
        ),
    };
    let heading = user
        .lang
        .get1("template-applied-to", "device", device.name.as_str());
    Ok(Html(
        apply_and_render(&state, &user, device, choice, heading).await?,
    ))
}

/// Options for the "new device" form: none, detect, or a specific template.
pub async fn device_form_options(
    state: &AppState,
    t: Lang,
    selected: &str,
) -> Result<Vec<TemplateOption>> {
    let templates = store::list_templates(&state.pool).await?;
    if templates.is_empty() {
        return Ok(Vec::new());
    }
    let mut options = vec![
        TemplateOption {
            value: String::new(),
            label: t.get("template-option-none"),
            selected: selected.is_empty(),
        },
        TemplateOption {
            value: "auto".to_owned(),
            label: t.get("template-option-auto"),
            selected: selected == "auto",
        },
    ];
    options.extend(templates.into_iter().map(|t| TemplateOption {
        selected: selected == t.id.to_string(),
        value: t.id.to_string(),
        label: t.name,
    }));
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{DeviceInput, Polling};
    use crate::snmp::SYS_OBJECT_ID;
    use crate::snmp::testagent::{FakeAgent, FakeValue};
    use crate::status::Thresholds;
    use crate::storage::devices::save_device;
    use crate::storage::sensors::{create_sensor_with_state, list_sensors, set_thresholds};
    use crate::web::testutil::*;

    const IF_DESCR: &[u64] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 2];
    const IF_TYPE: &[u64] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 3];
    const IF_OPER: &[u64] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 8];
    const MODEL: &[u64] = &[1, 3, 6, 1, 4, 1, 11863, 1, 1, 11];
    const POE: &[u64] = &[1, 3, 6, 1, 4, 1, 11863, 6, 56, 1, 1, 2, 1, 7];

    /// A switch with ports 1..=n (`up` lists the up ones) and PoE power on the first `poe` ports.
    async fn switch(n: u64, up: &[u64], poe: u64) -> FakeAgent {
        let agent = FakeAgent::spawn("public", "Fake switch", 100).await;
        agent.set(SYS_OBJECT_ID, FakeValue::ObjectId(MODEL.to_vec()));
        for port in 1..=n {
            let index = 49152 + port;
            agent.set(
                &[IF_DESCR, &[index]].concat(),
                FakeValue::Text(format!("gigabitEthernet 1/0/{port}")),
            );
            agent.set(&[IF_TYPE, &[index]].concat(), FakeValue::Integer(6));
            let status = if up.contains(&port) { 1 } else { 2 };
            agent.set(&[IF_OPER, &[index]].concat(), FakeValue::Integer(status));
        }
        for port in 1..=poe {
            agent.set(&[POE, &[port]].concat(), FakeValue::Integer(40));
        }
        agent
    }

    async fn device_on(h: &Harness, name: &str, port: u16, profile: i64) -> i64 {
        save_device(
            &h.state.pool,
            None,
            DeviceInput {
                name: name.to_owned(),
                host: "127.0.0.1".to_owned(),
                group_id: None,
                profile_id: Some(profile),
                polling: Polling {
                    port,
                    interval_secs: 60,
                    timeout_secs: 1,
                    retries: 0,
                },
            },
        )
        .await
        .unwrap()
    }

    fn poe_spec(port: u64) -> SensorSpec {
        SensorSpec::Oid {
            oid: [POE, &[port]].concat(),
            mode: crate::sensors::snmp::OidMode::Gauge,
            scale: 0.1,
            unit: Some("W".to_owned()),
        }
    }

    /// A configured source device: ping with a warning limit, uptime, traffic on ports 1 and 2,
    /// inactive PoE sensors for ports 1 and 3.
    async fn configured_source(h: &Harness, port: u16, profile: i64) -> i64 {
        let dev = device_on(h, "Switch Büro", port, profile).await;
        let pool = &h.state.pool;
        let ping = create_sensor_with_state(pool, dev, "Ping", &SensorSpec::Ping, true)
            .await
            .unwrap();
        let channel = crate::storage::sensors::get_sensor(pool, ping)
            .await
            .unwrap()
            .unwrap()
            .channels[0]
            .id;
        set_thresholds(
            pool,
            ping,
            &[(
                channel,
                Thresholds {
                    warn_above: Some(50.0),
                    ..Thresholds::default()
                },
            )],
        )
        .await
        .unwrap();
        create_sensor_with_state(pool, dev, "Uptime", &SensorSpec::Uptime, true)
            .await
            .unwrap();
        for (port, name) in [(1, "Uplink"), (2, "Traffic gigabitEthernet 1/0/2")] {
            let spec = SensorSpec::InterfaceTraffic {
                if_index: 49152 + port,
            };
            create_sensor_with_state(pool, dev, name, &spec, true)
                .await
                .unwrap();
        }
        for port in [1, 3] {
            let name = format!("PoE-Leistung Port {port}");
            create_sensor_with_state(pool, dev, &name, &poe_spec(port), false)
                .await
                .unwrap();
        }
        dev
    }

    fn device_form(csrf: &str, name: &str, port: u16, profile: i64, template: &str) -> String {
        format!(
            "csrf_token={csrf}&name={name}&host=127.0.0.1&group_id=&profile_id={profile}&port={port}\
             &interval_secs=60&timeout_secs=1&retries=0&template={template}"
        )
    }

    #[tokio::test]
    async fn a_device_becomes_a_template_that_sets_up_the_next_device() {
        let source = switch(4, &[1, 2], 4).await;
        let h = harness(true).await;
        let profile = add_v2c_profile(&h, "Fake", "public").await;
        let dev = configured_source(&h, source.port, profile).await;
        let (admin, csrf) = login(&h, "anna").await;

        let page = send(
            &h.app,
            get(&format!("/devices/{dev}/template/new"), Some(&admin)),
        )
        .await;
        assert_eq!(page.status, StatusCode::OK);
        for needle in [
            "value=\"1.3.6.1.4.1.11863.1.1.11\"",
            "Am Gerät: gigabitEthernet 1/0/1, gigabitEthernet 1/0/2",
            "name=\"i_1\" value=\"on\" aria-label=\"Ping übernehmen\" checked",
            "PoE-Leistung Port 3",
        ] {
            assert!(page.body.contains(needle), "{needle}: {}", page.body);
        }

        let reply = send(
            &h.app,
            post(
                &format!("/devices/{dev}/template"),
                Some(&admin),
                &format!(
                    "csrf_token={csrf}&name=TP-Link+SG2008&object_id=1.3.6.1.4.1.11863.1.1.11\
                     &traffic=same&status=up&i_1=on&i_2=on&i_5=on&i_6=on"
                ),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.body);
        assert_eq!(reply.location(), "/templates/1?msg=template_created");
        let template = store::get_template(&h.state.pool, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(template.source, "Switch Büro");
        assert_eq!(
            template.body.summary(Lang::De),
            "Ping, Uptime, Traffic: 2 bestimmte Ports, Status: aktive Ports, 2 eigene Werte"
        );
        let detail = send(&h.app, get(reply.location(), Some(&admin))).await;
        assert!(
            detail.body.contains("Traffic an „gigabitEthernet 1/0/1“")
                && detail.body.contains("Vorlage gespeichert.")
        );

        // A second switch of the same model: port 2 is down, PoE only on port 1.
        let target = switch(3, &[1, 3], 1).await;
        let page = send(&h.app, get("/devices/new", Some(&admin))).await;
        assert!(page.body.contains("<option value=\"auto\" selected>"));
        let result = send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &device_form(&csrf, "Switch+Keller", target.port, profile, "auto"),
            ),
        )
        .await;
        assert_eq!(result.status, StatusCode::OK);
        assert!(
            result.body.contains("Gerät „Switch Keller“ angelegt"),
            "{}",
            result.body
        );
        assert!(result.body.contains("Vorlage „TP-Link SG2008“"));
        assert!(
            result.body.contains("7 Sensoren angelegt, 1 übersprungen."),
            "{}",
            result.body
        );
        assert!(
            result
                .body
                .contains("PoE-Leistung Port 3: Das Gerät liefert")
        );

        let new_id = devices::list_devices(&h.state.pool)
            .await
            .unwrap()
            .into_iter()
            .find(|d| d.name == "Switch Keller")
            .unwrap()
            .id;
        let created = list_sensors(&h.state.pool, new_id).await.unwrap();
        let summary: Vec<(&str, bool)> = created
            .iter()
            .map(|s| (s.name.as_str(), s.enabled))
            .collect();
        assert_eq!(
            summary,
            [
                ("Ping", true),
                ("Uptime", true),
                ("Traffic gigabitEthernet 1/0/2", true),
                ("Uplink", true),
                ("Status gigabitEthernet 1/0/1", true),
                ("Status gigabitEthernet 1/0/3", true),
                ("PoE-Leistung Port 1", false),
            ]
        );
        assert_eq!(created[0].channels[0].thresholds.warn_above, Some(50.0));
        assert_eq!(
            store::template_of_device(&h.state.pool, new_id)
                .await
                .unwrap(),
            Some(1)
        );
        let sensors_page = send(
            &h.app,
            get(&format!("/devices/{new_id}/sensors"), Some(&admin)),
        )
        .await;
        assert!(
            sensors_page
                .body
                .contains("Eingerichtet mit „TP-Link SG2008“")
        );

        // Applying again adds nothing.
        let page = send(
            &h.app,
            get(&format!("/devices/{new_id}/template/apply"), Some(&admin)),
        )
        .await;
        assert!(
            page.body
                .contains("Passend zur Gerätekennung: TP-Link SG2008.")
        );
        let again = send(
            &h.app,
            post(
                &format!("/devices/{new_id}/template/apply"),
                Some(&admin),
                &format!("csrf_token={csrf}&template_id=1"),
            ),
        )
        .await;
        assert!(
            again
                .body
                .contains("0 Sensoren angelegt, 7 schon vorhanden, 1 übersprungen."),
            "{}",
            again.body
        );

        let list = send(&h.app, get("/templates", Some(&admin))).await;
        assert!(list.body.contains("TP-Link SG2008") && list.body.contains("aus Switch Büro"));
    }

    #[tokio::test]
    async fn without_a_match_or_a_device_nothing_is_created() {
        let h = harness(true).await;
        let profile = add_v2c_profile(&h, "Fake", "public").await;
        let source = switch(2, &[1], 0).await;
        let dev = configured_source(&h, source.port, profile).await;
        let (admin, csrf) = login(&h, "anna").await;
        send(
            &h.app,
            post(
                &format!("/devices/{dev}/template"),
                Some(&admin),
                &format!(
                    "csrf_token={csrf}&name=Ohne+Kennung&object_id=&traffic=up&status=skip&i_1=on"
                ),
            ),
        )
        .await;

        // Detection finds nothing: the device is created, the page says why no sensors came.
        let other = switch(2, &[1], 0).await;
        let result = send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &device_form(&csrf, "Neu", other.port, profile, "auto"),
            ),
        )
        .await;
        assert!(
            result
                .body
                .contains("Keine Vorlage passt zur Gerätekennung 1.3.6.1.4.1.11863.1.1.11")
        );

        // An unreachable device: same, with the SNMP explanation.
        let silent = FakeAgent::spawn("other-community", "x", 1).await;
        let result = send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &device_form(&csrf, "Stumm", silent.port, profile, "1"),
            ),
        )
        .await;
        assert!(
            result
                .body
                .contains("Die Vorlage wurde nicht angewendet. Keine Antwort"),
            "{}",
            result.body
        );
        let page = send(&h.app, get("/devices/3/template/new", Some(&admin))).await;
        assert!(
            page.body
                .contains("konnte nicht vollständig gelesen werden")
        );
        for id in [2, 3] {
            assert!(list_sensors(&h.state.pool, id).await.unwrap().is_empty());
        }

        // Without a template choice the old flow stays.
        let reply = send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &device_form(&csrf, "Schlicht", other.port, profile, ""),
            ),
        )
        .await;
        assert_eq!(reply.location(), "/?msg=device_created");
    }

    #[tokio::test]
    async fn templates_are_renamed_deleted_and_only_for_admins() {
        let h = harness(true).await;
        let profile = add_v2c_profile(&h, "Fake", "public").await;
        let source = switch(1, &[1], 0).await;
        let dev = configured_source(&h, source.port, profile).await;
        let (admin, csrf) = login(&h, "anna").await;
        let reply = send(
            &h.app,
            post(
                &format!("/devices/{dev}/template"),
                Some(&admin),
                &format!("csrf_token={csrf}&name=A&object_id=&traffic=skip&status=skip"),
            ),
        )
        .await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "an empty template is refused"
        );
        assert!(reply.body.contains("Die Vorlage wäre leer"));
        send(
            &h.app,
            post(
                &format!("/devices/{dev}/template"),
                Some(&admin),
                &format!("csrf_token={csrf}&name=A&object_id=&traffic=skip&status=skip&i_1=on"),
            ),
        )
        .await;
        let bad = send(
            &h.app,
            post(
                "/templates/1",
                Some(&admin),
                &format!("csrf_token={csrf}&name=B&object_id=abc"),
            ),
        )
        .await;
        assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(bad.body.contains("numerische OID") && bad.body.contains("value=\"abc\""));
        let ok = send(
            &h.app,
            post(
                "/templates/1",
                Some(&admin),
                &format!("csrf_token={csrf}&name=B&object_id=.1.3.6.1.4.1.9"),
            ),
        )
        .await;
        assert_eq!(ok.location(), "/templates/1?msg=template_saved");
        let stored = store::get_template(&h.state.pool, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (stored.name.as_str(), stored.object_id.as_deref()),
            ("B", Some("1.3.6.1.4.1.9"))
        );

        let (viewer, vcsrf) = login(&h, "bert").await;
        for request in [
            get("/templates", Some(&viewer)),
            get("/templates/1", Some(&viewer)),
            get(&format!("/devices/{dev}/template/new"), Some(&viewer)),
            get(&format!("/devices/{dev}/template/apply"), Some(&viewer)),
            post(
                "/templates/1/delete",
                Some(&viewer),
                &format!("csrf_token={vcsrf}"),
            ),
            post(
                &format!("/devices/{dev}/template/apply"),
                Some(&viewer),
                &format!("csrf_token={vcsrf}&template_id=1"),
            ),
        ] {
            assert_eq!(send(&h.app, request).await.status, StatusCode::FORBIDDEN);
        }

        let reply = send(
            &h.app,
            post(
                "/templates/1/delete",
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(reply.location(), "/templates?msg=template_deleted");
        assert_eq!(
            store::template_of_device(&h.state.pool, dev).await.unwrap(),
            None
        );
        assert_eq!(
            list_sensors(&h.state.pool, dev).await.unwrap().len(),
            6,
            "sensors stay"
        );
    }

    #[tokio::test]
    async fn the_list_is_shown_in_english() {
        let h = harness(true).await;
        crate::storage::users::set_language(&h.state.pool, 1, Some(Lang::En))
            .await
            .unwrap();
        let (admin, _) = login(&h, "anna").await;
        let page = send(&h.app, get("/templates", Some(&admin))).await;
        for text in [
            "<h1>Device templates</h1>",
            "No templates yet.",
            "How templates work",
        ] {
            assert!(page.body.contains(text), "{text} missing");
        }
    }
}
