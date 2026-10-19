//! Device pages (admin only): create, edit, delete and the SNMP connection test.

use std::time::Duration;

use askama::Template;
use axum::Form;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::devices::{
    DEFAULT_INTERVAL_SECS, DEFAULT_PORT, DEFAULT_RETRIES, DEFAULT_TIMEOUT_SECS, Device,
    DeviceInput, Polling,
};
use crate::error::{Error, Result};
use crate::i18n::{Lang, Msg};
use crate::sensors::spec::SensorSpec;
use crate::snmp::{self, SnmpTarget, format_uptime};
use crate::storage::{credentials, devices as store, groups, sensors};
use crate::web::auth::{CsrfForm, CurrentUser};
use crate::web::{AppState, Layout, SelectOption, group_options};

/// The test never blocks a request for long, whatever the device is configured to.
const TEST_MAX_TIMEOUT_SECS: u32 = 10;
const TEST_MAX_RETRIES: u32 = 1;

/// Values echoed back into the form.
#[derive(Clone)]
struct FormValues {
    name: String,
    host: String,
    port: String,
    interval_secs: String,
    timeout_secs: String,
    retries: String,
    /// Template choice when creating: empty, `auto` or a template id.
    template: String,
}

impl Default for FormValues {
    fn default() -> Self {
        Self {
            name: String::new(),
            host: String::new(),
            port: DEFAULT_PORT.to_string(),
            interval_secs: DEFAULT_INTERVAL_SECS.to_string(),
            timeout_secs: DEFAULT_TIMEOUT_SECS.to_string(),
            retries: DEFAULT_RETRIES.to_string(),
            template: "auto".to_owned(),
        }
    }
}

impl FormValues {
    fn from_device(d: &Device) -> Self {
        Self {
            name: d.name.clone(),
            host: d.host.clone(),
            port: d.polling.port.to_string(),
            interval_secs: d.polling.interval_secs.to_string(),
            timeout_secs: d.polling.timeout_secs.to_string(),
            retries: d.polling.retries.to_string(),
            template: String::new(),
        }
    }
}

#[derive(Template)]
#[template(path = "device_form.html")]
struct FormTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    /// Catalog id of the heading.
    heading: &'static str,
    action: String,
    test_action: Option<String>,
    delete_action: Option<String>,
    error: Option<String>,
    form: FormValues,
    groups: Vec<SelectOption>,
    profiles: Vec<SelectOption>,
    /// "Kein SNMP – nur Ping" is selected.
    no_snmp: bool,
    /// Device templates to apply right after creating (empty when editing or without templates).
    templates: Vec<crate::web::templates::TemplateOption>,
}

pub struct ProbeView {
    sys_descr: String,
    uptime: String,
    round_trip_ms: u128,
}

#[derive(Template)]
#[template(path = "device_test.html")]
struct TestTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    device: Device,
    profile_summary: String,
    success: Option<ProbeView>,
    failure: Option<String>,
}

/// Deliberately no `Debug`, consistent with the other forms.
#[derive(Deserialize)]
pub struct DeviceForm {
    csrf_token: String,
    name: String,
    host: String,
    #[serde(default)]
    group_id: String,
    #[serde(default)]
    profile_id: String,
    port: String,
    interval_secs: String,
    timeout_secs: String,
    retries: String,
    #[serde(default)]
    template: String,
}

#[derive(Deserialize)]
pub struct DeleteForm {
    csrf_token: String,
    confirm: Option<String>,
}

fn number<T: std::str::FromStr>(value: &str, invalid: &'static str) -> Result<T> {
    value
        .trim()
        .parse()
        .map_err(|_| Error::invalid(Msg::new(invalid)))
}

impl DeviceForm {
    fn values(&self) -> FormValues {
        FormValues {
            name: self.name.clone(),
            host: self.host.clone(),
            port: self.port.clone(),
            interval_secs: self.interval_secs.clone(),
            timeout_secs: self.timeout_secs.clone(),
            retries: self.retries.clone(),
            template: self.template.clone(),
        }
    }

    fn selected_group(&self) -> Option<i64> {
        self.group_id.trim().parse().ok()
    }

    fn selected_profile(&self) -> Option<i64> {
        self.profile_id.trim().parse().ok()
    }

    fn to_input(&self) -> Result<DeviceInput> {
        let group_id = match self.group_id.trim() {
            "" => None,
            text => Some(
                text.parse()
                    .map_err(|_| Error::invalid(Msg::new("group-invalid")))?,
            ),
        };
        let profile_id = match self.profile_id.trim() {
            "none" => None,
            text => Some(
                text.parse()
                    .map_err(|_| Error::invalid(Msg::new("device-profile-missing")))?,
            ),
        };
        Ok(DeviceInput {
            name: self.name.clone(),
            host: self.host.clone(),
            group_id,
            profile_id,
            polling: Polling {
                port: number(&self.port, "polling-port-not-number")?,
                interval_secs: number(&self.interval_secs, "polling-interval-not-number")?,
                timeout_secs: number(&self.timeout_secs, "polling-timeout-not-number")?,
                retries: number(&self.retries, "polling-retries-not-number")?,
            },
        })
    }
}

async fn profile_options(
    state: &AppState,
    user: &CurrentUser,
    selected: Option<i64>,
) -> Result<Vec<SelectOption>> {
    Ok(credentials::list_profiles(&state.pool)
        .await?
        .into_iter()
        .map(|p| SelectOption {
            label: format!("{} ({})", p.name, p.summary(user.lang)),
            selected: selected == Some(p.id),
            id: p.id,
        })
        .collect())
}

async fn render_form(
    state: &AppState,
    user: &CurrentUser,
    existing: Option<&Device>,
    form: FormValues,
    selected_group: Option<i64>,
    selected_profile: Option<i64>,
    error: Option<String>,
) -> Result<String> {
    let all_groups = groups::list_groups(&state.pool).await?;
    let templates = if existing.is_none() {
        crate::web::templates::device_form_options(state, user.lang, &form.template).await?
    } else {
        Vec::new()
    };
    let page = FormTemplate {
        no_snmp: selected_profile.is_none(),
        templates,
        layout: user.layout(),
        t: user.lang,
        heading: if existing.is_some() {
            "device-edit"
        } else {
            "device-new"
        },
        action: existing.map_or_else(|| "/devices".to_owned(), |d| format!("/devices/{}", d.id)),
        test_action: existing.map(|d| format!("/devices/{}/test", d.id)),
        delete_action: existing.map(|d| format!("/devices/{}/delete", d.id)),
        error,
        form,
        groups: group_options(&all_groups, selected_group, &[]),
        profiles: profile_options(state, user, selected_profile).await?,
    };
    Ok(page.render()?)
}

pub async fn new_form(State(state): State<AppState>, user: CurrentUser) -> Result<Html<String>> {
    user.require_admin()?;
    let first_profile = credentials::list_profiles(&state.pool)
        .await?
        .first()
        .map(|p| p.id);
    Ok(Html(
        render_form(
            &state,
            &user,
            None,
            FormValues::default(),
            None,
            first_profile,
            None,
        )
        .await?,
    ))
}

pub async fn edit_form(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let device = store::get_device(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)?;
    let values = FormValues::from_device(&device);
    Ok(Html(
        render_form(
            &state,
            &user,
            Some(&device),
            values,
            device.group_id,
            device.profile_id,
            None,
        )
        .await?,
    ))
}

pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Form(form): Form<DeviceForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    save(&state, &user, None, &form).await
}

pub async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<DeviceForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    save(&state, &user, Some(id), &form).await
}

async fn save(
    state: &AppState,
    user: &CurrentUser,
    id: Option<i64>,
    form: &DeviceForm,
) -> Result<Response> {
    let outcome = match form.to_input() {
        Ok(input) => match snmp_sensors_left(state, id, &input).await {
            Ok(()) => store::save_device(&state.pool, id, input).await,
            Err(e) => Err(e),
        },
        Err(e) => Err(e),
    };
    match outcome {
        Ok(saved) if id.is_none() && form.profile_id.trim() == "none" => {
            // A ping-only device gets its only sensor right away.
            sensors::create_sensor(&state.pool, saved, "Ping", &SensorSpec::Ping).await?;
            tracing::info!(user = %user.username, device = saved, "ping-only device created");
            Ok(
                Redirect::to(&format!("/devices/{saved}/sensors?msg=device_created_ping"))
                    .into_response(),
            )
        }
        Ok(saved) => {
            tracing::info!(user = %user.username, device = saved, "device saved");
            let choice = match form.template.trim() {
                _ if id.is_some() => None,
                "" => None,
                "auto" => Some(crate::web::templates::Choice::Detect),
                text => text.parse().ok().map(crate::web::templates::Choice::Id),
            };
            if let Some(choice) = choice {
                let device = store::get_device(&state.pool, saved)
                    .await?
                    .ok_or(Error::NotFound)?;
                let heading = user.lang.get1("device-created-named", "name", &device.name);
                let html =
                    crate::web::templates::apply_and_render(state, user, device, choice, heading)
                        .await?;
                return Ok(Html(html).into_response());
            }
            let msg = if id.is_some() {
                "device_updated"
            } else {
                "device_created"
            };
            Ok(Redirect::to(&format!("/?msg={msg}")).into_response())
        }
        Err(Error::Invalid(message) | Error::Conflict(message)) => {
            let existing = match id {
                Some(id) => Some(
                    store::get_device(&state.pool, id)
                        .await?
                        .ok_or(Error::NotFound)?,
                ),
                None => None,
            };
            let html = render_form(
                state,
                user,
                existing.as_ref(),
                form.values(),
                form.selected_group(),
                form.selected_profile(),
                Some(user.lang.msg(&message)),
            )
            .await?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response())
        }
        Err(e) => Err(e),
    }
}

/// A device can only drop its SNMP profile once it has no SNMP sensors left.
async fn snmp_sensors_left(state: &AppState, id: Option<i64>, input: &DeviceInput) -> Result<()> {
    let (Some(id), None) = (id, input.profile_id) else {
        return Ok(());
    };
    let left = sensors::list_sensors(&state.pool, id)
        .await?
        .iter()
        .filter(|s| s.spec.uses_snmp())
        .count();
    if left > 0 {
        return Err(Error::invalid(
            Msg::new("device-snmp-sensors-left").with("count", left),
        ));
    }
    Ok(())
}

pub async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<DeleteForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    if form.confirm.as_deref() != Some("yes") {
        return Err(Error::invalid(Msg::new("confirm-delete")));
    }
    if !store::delete_device(&state.pool, id).await? {
        return Err(Error::NotFound);
    }
    tracing::info!(user = %user.username, device = id, "device deleted");
    Ok(Redirect::to("/?msg=device_deleted").into_response())
}

/// Asks the device for `sysDescr`/`sysUpTime` with its stored profile and shows the result.
pub async fn test_connection(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Result<Html<String>> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let device = store::get_device(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)?;
    let credentials = crate::web::snmp_credentials(&state, &device).await?;
    let profile_summary =
        credentials::get_profile(&state.pool, device.profile_id.unwrap_or_default())
            .await?
            .map(|p| p.summary(user.lang))
            .unwrap_or_default();

    let target = SnmpTarget {
        host: device.host.clone(),
        port: device.polling.port,
        timeout: Duration::from_secs(u64::from(
            device.polling.timeout_secs.min(TEST_MAX_TIMEOUT_SECS),
        )),
        retries: device.polling.retries.min(TEST_MAX_RETRIES),
    };
    let outcome = snmp::probe(&target, &credentials).await;
    tracing::info!(user = %user.username, device = id, ok = outcome.is_ok(), "connection test");

    let (success, failure) = match outcome {
        Ok(result) => (
            Some(ProbeView {
                sys_descr: result.sys_descr,
                uptime: format_uptime(user.lang, result.uptime_ticks),
                round_trip_ms: result.round_trip.as_millis(),
            }),
            None,
        ),
        Err(e) => (None, Some(user.lang.msg(&e.user_message()))),
    };
    let page = TestTemplate {
        layout: user.layout(),
        t: user.lang,
        device,
        profile_summary,
        success,
        failure,
    };
    Ok(Html(page.render()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snmp::testagent::FakeAgent;
    use crate::web::testutil::*;

    #[allow(clippy::too_many_arguments)]
    fn form(
        csrf: &str,
        name: &str,
        host: &str,
        group: &str,
        profile: i64,
        port: u16,
        interval: u32,
        timeout: u32,
        retries: u32,
    ) -> String {
        format!(
            "csrf_token={csrf}&name={name}&host={host}&group_id={group}&profile_id={profile}\
             &port={port}&interval_secs={interval}&timeout_secs={timeout}&retries={retries}"
        )
    }

    fn simple(csrf: &str, name: &str, host: &str, profile: i64) -> String {
        form(csrf, name, host, "", profile, 161, 60, 5, 1)
    }

    #[tokio::test]
    async fn only_admins_can_manage_devices_but_viewers_see_the_overview() {
        let h = harness(true).await;
        let profile = add_v2c_profile(&h, "Alt", "public").await;
        let (admin, acsrf) = login(&h, "anna").await;
        send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &simple(&acsrf, "Switch+Keller", "192.168.178.251", profile),
            ),
        )
        .await;

        for path in ["/devices/new", "/devices/1/edit"] {
            assert_eq!(
                send(&h.app, get(path, None)).await.status,
                StatusCode::SEE_OTHER,
                "{path}"
            );
        }
        let (viewer, vcsrf) = login(&h, "bert").await;
        for path in ["/devices/new", "/devices/1/edit"] {
            assert_eq!(
                send(&h.app, get(path, Some(&viewer))).await.status,
                StatusCode::FORBIDDEN,
                "{path}"
            );
        }
        for (path, body) in [
            (
                "/devices".to_owned(),
                simple(&vcsrf, "X", "10.0.0.1", profile),
            ),
            (
                "/devices/1".to_owned(),
                simple(&vcsrf, "X", "10.0.0.1", profile),
            ),
            (
                "/devices/1/delete".to_owned(),
                format!("csrf_token={vcsrf}&confirm=yes"),
            ),
            ("/devices/1/test".to_owned(), format!("csrf_token={vcsrf}")),
        ] {
            assert_eq!(
                send(&h.app, post(&path, Some(&viewer), &body)).await.status,
                StatusCode::FORBIDDEN,
                "{path}"
            );
        }

        let home = send(&h.app, get("/", Some(&viewer))).await;
        assert!(home.body.contains("Switch Keller") && home.body.contains("192.168.178.251:161"));
        assert!(!home.body.contains("Testen") && !home.body.contains("/devices/1/edit"));
        assert!(
            !home.body.contains("Alt"),
            "viewers must not see profile names"
        );
        assert_eq!(store::list_devices(&h.state.pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn admin_creates_devices_in_groups_and_sees_them_in_the_tree() {
        let h = harness(true).await;
        let profile = add_v2c_profile(&h, "Omada", "public").await;
        let (admin, csrf) = login(&h, "anna").await;
        crate::storage::groups::create_group(&h.state.pool, "Keller", None)
            .await
            .unwrap();

        let body = form(
            &csrf,
            "Switch+Keller",
            "192.168.178.251",
            "1",
            profile,
            161,
            30,
            5,
            1,
        );
        let reply = send(&h.app, post("/devices", Some(&admin), &body)).await;
        assert_eq!(reply.location(), "/?msg=device_created");
        send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &simple(&csrf, "Router", "192.168.178.1", profile),
            ),
        )
        .await;

        let home = send(&h.app, get("/?msg=device_created", Some(&admin))).await;
        assert!(home.body.contains("Gerät angelegt."));
        let pos = |needle: &str| home.body.find(needle).unwrap();
        // Ungrouped devices first under their own heading, then the group with its device.
        assert!(
            pos("Ohne Gruppe") < pos("Router")
                && pos("Router") < pos("Keller")
                && pos("Keller") < pos("Switch Keller")
        );
        assert!(home.body.contains("30 s") && home.body.contains("Omada"));
        assert!(home.body.contains("/devices/1/edit") && home.body.contains("/devices/1/test"));
    }

    #[tokio::test]
    async fn validation_errors_keep_the_entered_values() {
        let h = harness(true).await;
        let profile = add_v2c_profile(&h, "Omada", "public").await;
        let (admin, csrf) = login(&h, "anna").await;

        let cases = [
            (
                simple(&csrf, "A", "bad+host", profile),
                "gültige IP-Adresse",
            ),
            (simple(&csrf, "A", "0.0.0.0", profile), "gültige IP-Adresse"),
            (
                form(&csrf, "A", "10.0.0.1", "", profile, 161, 5, 5, 1),
                "Intervall",
            ),
            (
                form(&csrf, "A", "10.0.0.1", "", profile, 161, 10, 5, 3),
                "nicht überschreiten",
            ),
            (
                form(&csrf, "A", "10.0.0.1", "", profile, 0, 60, 5, 1),
                "Port",
            ),
            (
                form(&csrf, "A", "10.0.0.1", "", profile, 161, 60, 5, 9),
                "Wiederholungen",
            ),
            (simple(&csrf, "A", "10.0.0.1", 99), "existiert nicht"),
            (
                form(&csrf, "A", "10.0.0.1", "77", profile, 161, 60, 5, 1),
                "existiert nicht",
            ),
            (simple(&csrf, "+", "10.0.0.1", profile), "Name"),
            (
                simple(&csrf, "A", "10.0.0.1", profile).replace("port=161", "port=abc"),
                "Ungültiger Wert",
            ),
            (
                simple(&csrf, "A", "10.0.0.1", profile).replace("group_id=", "group_id=xyz"),
                "Ungültige Gruppe",
            ),
        ];
        for (body, expected) in cases {
            let reply = send(&h.app, post("/devices", Some(&admin), &body)).await;
            assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{expected}");
            assert!(reply.body.contains(expected), "{expected}: {}", reply.body);
        }
        // Entered values are echoed back.
        let reply = send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &simple(&csrf, "Mein+Switch", "bad+host", profile),
            ),
        )
        .await;
        assert!(
            reply.body.contains("value=\"Mein Switch\"")
                && reply.body.contains("value=\"bad host\"")
        );
        assert!(store::list_devices(&h.state.pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn duplicate_names_and_missing_profiles_are_explained() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let new_page = send(&h.app, get("/devices/new", Some(&admin))).await;
        assert!(new_page.body.contains("noch kein SNMP-Profil"));

        let profile = add_v2c_profile(&h, "Omada", "public").await;
        send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &simple(&csrf, "A", "10.0.0.1", profile),
            ),
        )
        .await;
        let dup = send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &simple(&csrf, "a", "10.0.0.2", profile),
            ),
        )
        .await;
        assert_eq!(dup.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(dup.body.contains("existiert bereits"));
    }

    #[tokio::test]
    async fn edit_update_and_delete_work_with_csrf_and_confirmation() {
        let h = harness(true).await;
        let profile = add_v2c_profile(&h, "Omada", "public").await;
        let (admin, csrf) = login(&h, "anna").await;
        send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &simple(&csrf, "Alt", "10.0.0.1", profile),
            ),
        )
        .await;

        let edit = send(&h.app, get("/devices/1/edit", Some(&admin))).await;
        assert!(edit.body.contains("value=\"Alt\"") && edit.body.contains("value=\"10.0.0.1\""));
        assert!(edit.body.contains("Verbindung testen"));

        let bad = send(
            &h.app,
            post(
                "/devices/1",
                Some(&admin),
                &simple("falsch", "Neu", "10.0.0.2", profile),
            ),
        )
        .await;
        assert_eq!(bad.status, StatusCode::FORBIDDEN);
        let ok = send(
            &h.app,
            post(
                "/devices/1",
                Some(&admin),
                &simple(&csrf, "Neu", "10.0.0.2", profile),
            ),
        )
        .await;
        assert_eq!(ok.location(), "/?msg=device_updated");
        let d = store::get_device(&h.state.pool, 1).await.unwrap().unwrap();
        assert_eq!((d.name.as_str(), d.host.as_str()), ("Neu", "10.0.0.2"));
        assert_eq!(
            send(
                &h.app,
                post(
                    "/devices/99",
                    Some(&admin),
                    &simple(&csrf, "X", "10.0.0.3", profile)
                )
            )
            .await
            .status,
            StatusCode::NOT_FOUND
        );

        let unconfirmed = send(
            &h.app,
            post(
                "/devices/1/delete",
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(unconfirmed.status, StatusCode::UNPROCESSABLE_ENTITY);
        let deleted = send(
            &h.app,
            post(
                "/devices/1/delete",
                Some(&admin),
                &format!("csrf_token={csrf}&confirm=yes"),
            ),
        )
        .await;
        assert_eq!(deleted.location(), "/?msg=device_deleted");
        let gone = send(
            &h.app,
            post(
                "/devices/1/delete",
                Some(&admin),
                &format!("csrf_token={csrf}&confirm=yes"),
            ),
        )
        .await;
        assert_eq!(gone.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_profile_in_use_cannot_be_deleted_through_the_ui() {
        let h = harness(true).await;
        let profile = add_v2c_profile(&h, "Omada", "public").await;
        let (admin, csrf) = login(&h, "anna").await;
        send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &simple(&csrf, "A", "10.0.0.1", profile),
            ),
        )
        .await;
        let reply = send(
            &h.app,
            post(
                &format!("/credentials/{profile}/delete"),
                Some(&admin),
                &format!("csrf_token={csrf}&confirm=yes"),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::CONFLICT);
        assert!(reply.body.contains("noch von Geräten verwendet"));
        assert_eq!(
            crate::storage::credentials::list_profiles(&h.state.pool)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn connection_test_reports_success_against_a_working_agent() {
        let h = harness(true).await;
        let agent = FakeAgent::spawn(
            "geheim-community",
            "Omada 8-Port Gigabit Smart Switch",
            1_618_844_685,
        )
        .await;
        let profile = add_v2c_profile(&h, "Fake", "geheim-community").await;
        let (admin, csrf) = login(&h, "anna").await;
        let body = form(
            &csrf,
            "Fake",
            "127.0.0.1",
            "",
            profile,
            agent.port,
            60,
            2,
            0,
        );
        send(&h.app, post("/devices", Some(&admin), &body)).await;

        let reply = send(
            &h.app,
            post(
                "/devices/1/test",
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK);
        assert!(reply.body.contains("Verbindung erfolgreich"));
        assert!(reply.body.contains("Omada 8-Port Gigabit Smart Switch"));
        assert!(reply.body.contains("187 Tage 8 Std. 47 Min."));
        assert!(!reply.body.contains("geheim-community"));
    }

    #[tokio::test]
    async fn connection_test_explains_failures() {
        let h = harness(true).await;
        let agent = FakeAgent::spawn("richtig", "x", 1).await;
        let wrong = add_v2c_profile(&h, "Falsch", "falsch").await;
        let (admin, csrf) = login(&h, "anna").await;
        // Wrong community: the agent stays silent.
        send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &form(&csrf, "A", "127.0.0.1", "", wrong, agent.port, 60, 1, 0),
            ),
        )
        .await;
        let reply = send(
            &h.app,
            post(
                "/devices/1/test",
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK);
        assert!(reply.body.contains("Keine Antwort"));
        assert!(!reply.body.contains("Verbindung erfolgreich"));
        assert!(!reply.body.contains("falsch</"), "community leaked");

        // Unresolvable host.
        send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &form(
                    &csrf,
                    "B",
                    "gibt-es-nicht.invalid",
                    "",
                    wrong,
                    161,
                    60,
                    1,
                    0,
                ),
            ),
        )
        .await;
        let reply = send(
            &h.app,
            post(
                "/devices/2/test",
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert!(reply.body.contains("nicht erreichbar"));
    }

    #[tokio::test]
    async fn connection_test_needs_a_valid_csrf_token_and_an_existing_device() {
        let h = harness(true).await;
        let profile = add_v2c_profile(&h, "P", "public").await;
        let (admin, csrf) = login(&h, "anna").await;
        send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &simple(&csrf, "A", "10.0.0.1", profile),
            ),
        )
        .await;
        assert_eq!(
            send(
                &h.app,
                post("/devices/1/test", Some(&admin), "csrf_token=falsch")
            )
            .await
            .status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(
                &h.app,
                post(
                    "/devices/99/test",
                    Some(&admin),
                    &format!("csrf_token={csrf}")
                )
            )
            .await
            .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(&h.app, get("/devices/1/test", Some(&admin)))
                .await
                .status,
            StatusCode::METHOD_NOT_ALLOWED
        );
    }

    #[tokio::test]
    async fn device_names_are_html_escaped_in_the_overview() {
        let h = harness(true).await;
        let profile = add_v2c_profile(&h, "P", "public").await;
        let (admin, csrf) = login(&h, "anna").await;
        send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &simple(&csrf, "%3Cscript%3Ex%3C%2Fscript%3E", "10.0.0.1", profile),
            ),
        )
        .await;
        let home = send(&h.app, get("/", Some(&admin))).await;
        assert!(!home.body.contains("<script>x"));
    }

    fn ping_only(csrf: &str, name: &str, host: &str) -> String {
        format!(
            "csrf_token={csrf}&name={name}&host={host}&group_id=&profile_id=none&port=161\
             &interval_secs=60&timeout_secs=5&retries=1&template=auto"
        )
    }

    #[tokio::test]
    async fn devices_without_snmp_get_a_ping_sensor_and_no_snmp_actions() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        // Works even before any SNMP profile exists.
        let page = send(&h.app, get("/devices/new", Some(&admin))).await;
        assert!(
            page.body
                .contains("<option value=\"none\" selected>Kein SNMP – nur Ping</option>")
        );
        let reply = send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &ping_only(&csrf, "Drucker", "10.0.0.9"),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.body);
        assert_eq!(
            reply.location(),
            "/devices/1/sensors?msg=device_created_ping"
        );
        let device = store::get_device(&h.state.pool, 1).await.unwrap().unwrap();
        assert_eq!(
            (device.profile_id, device.profile_label(Lang::De)),
            (None, "kein SNMP".to_owned())
        );
        let sensors = sensors::list_sensors(&h.state.pool, 1).await.unwrap();
        assert_eq!(sensors.len(), 1);
        assert_eq!(
            (sensors[0].name.as_str(), &sensors[0].spec),
            ("Ping", &SensorSpec::Ping)
        );

        let page = send(&h.app, get(reply.location(), Some(&admin))).await;
        assert!(
            page.body.contains("Gerät ohne SNMP angelegt")
                && page.body.contains("kein SNMP, nur Ping")
        );
        for absent in [
            "/test\"",
            "/values\"",
            "/discover\"",
            "/template/apply",
            "value=\"uptime\"",
            "value=\"oid\"",
        ] {
            assert!(!page.body.contains(absent), "{absent}");
        }
        let home = send(&h.app, get("/", Some(&admin))).await;
        assert!(home.body.contains("· kein SNMP") && !home.body.contains("/devices/1/test"));

        // SNMP actions are refused with an explanation.
        for (path, body) in [
            (
                "/devices/1/sensors",
                format!("csrf_token={csrf}&kind=uptime&name="),
            ),
            ("/devices/1/test", format!("csrf_token={csrf}")),
            ("/devices/1/discover", format!("csrf_token={csrf}")),
            ("/devices/1/values", format!("csrf_token={csrf}")),
        ] {
            let reply = send(&h.app, post(path, Some(&admin), &body)).await;
            assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{path}");
            assert!(reply.body.contains("SNMP"), "{path}: {}", reply.body);
        }

        // A profile can be added later; dropping it again needs the SNMP sensors gone.
        let profile = add_v2c_profile(&h, "Omada", "public").await;
        let with_profile = ping_only(&csrf, "Drucker", "10.0.0.9")
            .replace("profile_id=none", &format!("profile_id={profile}"));
        assert_eq!(
            send(&h.app, post("/devices/1", Some(&admin), &with_profile))
                .await
                .status,
            StatusCode::SEE_OTHER
        );
        sensors::create_sensor(&h.state.pool, 1, "Uptime", &SensorSpec::Uptime)
            .await
            .unwrap();
        let refused = send(
            &h.app,
            post(
                "/devices/1",
                Some(&admin),
                &ping_only(&csrf, "Drucker", "10.0.0.9"),
            ),
        )
        .await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(refused.body.contains("noch einen SNMP-Sensor"));
        let uptime = sensors::list_sensors(&h.state.pool, 1)
            .await
            .unwrap()
            .into_iter()
            .find(|s| s.name == "Uptime")
            .unwrap();
        sensors::delete_sensor(&h.state.pool, uptime.id)
            .await
            .unwrap();
        assert_eq!(
            send(
                &h.app,
                post(
                    "/devices/1",
                    Some(&admin),
                    &ping_only(&csrf, "Drucker", "10.0.0.9")
                )
            )
            .await
            .status,
            StatusCode::SEE_OTHER
        );
        assert_eq!(
            store::get_device(&h.state.pool, 1)
                .await
                .unwrap()
                .unwrap()
                .profile_id,
            None
        );
    }

    #[tokio::test]
    async fn device_pages_and_their_errors_are_shown_in_english() {
        let h = harness(true).await;
        crate::storage::users::set_language(&h.state.pool, 1, Some(Lang::En))
            .await
            .unwrap();
        let (admin, csrf) = login(&h, "anna").await;
        let form = send(&h.app, get("/devices/new", Some(&admin))).await;
        assert!(form.body.contains("<h1>New device</h1>"));
        assert!(form.body.contains("No SNMP – ping only"));
        let bad = ping_only(&csrf, "Drucker", "300.1.1.1");
        let refused = send(&h.app, post("/devices", Some(&admin), &bad)).await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            refused
                .body
                .contains("Please enter a valid IP address or host name.")
        );

        let created = send(
            &h.app,
            post(
                "/devices",
                Some(&admin),
                &ping_only(&csrf, "Drucker", "10.0.0.9"),
            ),
        )
        .await;
        assert_eq!(created.status, StatusCode::SEE_OTHER);
        let home = send(&h.app, get("/?msg=device_created", Some(&admin))).await;
        for text in [
            "Device created.",
            "1 device",
            "every 60 s",
            "no SNMP",
            "All devices",
            "Inactive / unknown",
        ] {
            assert!(home.body.contains(text), "{text}");
        }
        let group = send(&h.app, get("/groups/new", Some(&admin))).await;
        assert!(group.body.contains("<h1>New group</h1>") && group.body.contains("– top level –"));
    }
}
