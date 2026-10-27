//! SNMP credential profile pages (admin only). Passwords are never rendered; forms show only "set".

use askama::Template;
use axum::Form;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::audit::{Action, Changes, Kind};
use crate::credentials::{
    AuthProtocol, PrivProtocol, ProfileInput, ProfileKind, ProfileView, Secret, SecurityLevel,
    SnmpVersion,
};
use crate::error::{Error, Result};
use crate::i18n::{Lang, Msg};
use crate::storage::credentials as store;
use crate::web::audit::{self, Object};
use crate::web::auth::CurrentUser;
use crate::web::{AppState, Layout};

#[derive(Template)]
#[template(path = "credentials_list.html")]
struct ListTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    profiles: Vec<ProfileView>,
    flash: Option<String>,
}

/// Non-secret values that are echoed back into the form.
struct FormValues {
    name: String,
    version: String,
    username: String,
    level: String,
    auth_protocol: String,
    priv_protocol: String,
}

impl Default for FormValues {
    fn default() -> Self {
        Self {
            name: String::new(),
            version: SnmpVersion::V3.as_str().to_owned(),
            username: String::new(),
            level: SecurityLevel::AuthNoPriv.as_str().to_owned(),
            auth_protocol: AuthProtocol::Sha256.as_str().to_owned(),
            priv_protocol: String::new(),
        }
    }
}

impl FormValues {
    fn from_view(view: &ProfileView) -> Self {
        Self {
            name: view.name.clone(),
            version: view.version.as_str().to_owned(),
            username: view.username.clone().unwrap_or_default(),
            level: view
                .level
                .unwrap_or(SecurityLevel::AuthNoPriv)
                .as_str()
                .to_owned(),
            auth_protocol: view
                .auth_protocol
                .unwrap_or(AuthProtocol::Sha256)
                .as_str()
                .to_owned(),
            priv_protocol: view
                .priv_protocol
                .map(|p| p.as_str().to_owned())
                .unwrap_or_default(),
        }
    }
}

#[derive(Template)]
#[template(path = "credential_form.html")]
struct FormTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    heading: &'static str,
    action: String,
    delete_action: Option<String>,
    error: Option<String>,
    form: FormValues,
    versions: &'static [SnmpVersion],
    levels: &'static [SecurityLevel],
    auth_protocols: &'static [AuthProtocol],
    priv_protocols: &'static [PrivProtocol],
    community_set: bool,
    auth_password_set: bool,
    priv_password_set: bool,
}

impl FormTemplate {
    fn new(user: &CurrentUser, form: FormValues, existing: Option<&ProfileView>) -> Self {
        Self {
            layout: user.layout(),
            t: user.lang,
            heading: if existing.is_some() {
                "profile-edit"
            } else {
                "profile-new"
            },
            action: existing.map_or_else(
                || "/credentials".to_owned(),
                |p| format!("/credentials/{}", p.id),
            ),
            delete_action: existing.map(|p| format!("/credentials/{}/delete", p.id)),
            error: None,
            form,
            versions: SnmpVersion::ALL,
            levels: SecurityLevel::ALL,
            auth_protocols: AuthProtocol::ALL,
            priv_protocols: PrivProtocol::ALL,
            community_set: existing.is_some_and(|p| p.version == SnmpVersion::V2c),
            auth_password_set: existing.is_some_and(|p| p.version == SnmpVersion::V3),
            priv_password_set: existing.is_some_and(|p| p.priv_protocol.is_some()),
        }
    }
}

#[derive(Deserialize)]
pub struct ListQuery {
    msg: Option<String>,
}

/// Deliberately has no `Debug`: it carries passwords.
#[derive(Deserialize)]
pub struct ProfileForm {
    csrf_token: String,
    name: String,
    version: String,
    #[serde(default)]
    community: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    level: String,
    #[serde(default)]
    auth_protocol: String,
    #[serde(default)]
    auth_password: String,
    #[serde(default)]
    priv_protocol: String,
    #[serde(default)]
    priv_password: String,
}

#[derive(Deserialize)]
pub struct DeleteForm {
    csrf_token: String,
    confirm: Option<String>,
}

fn secret(value: &str) -> Option<Secret> {
    (!value.is_empty()).then(|| Secret::new(value))
}

fn invalid(id: &'static str) -> Error {
    Error::invalid(Msg::new(id))
}

impl ProfileForm {
    fn values(&self) -> FormValues {
        FormValues {
            name: self.name.clone(),
            version: self.version.clone(),
            username: self.username.clone(),
            level: self.level.clone(),
            auth_protocol: self.auth_protocol.clone(),
            priv_protocol: self.priv_protocol.clone(),
        }
    }

    fn to_input(&self) -> Result<ProfileInput> {
        let kind = match SnmpVersion::parse(&self.version)
            .ok_or_else(|| invalid("profile-version-invalid"))?
        {
            SnmpVersion::V2c => ProfileKind::V2c {
                community: secret(&self.community),
            },
            SnmpVersion::V3 => ProfileKind::V3 {
                username: self.username.trim().to_owned(),
                level: SecurityLevel::parse(&self.level)
                    .ok_or_else(|| invalid("profile-level-invalid"))?,
                auth_protocol: AuthProtocol::parse(&self.auth_protocol)
                    .ok_or_else(|| invalid("profile-auth-protocol-invalid"))?,
                auth_password: secret(&self.auth_password),
                priv_protocol: match self.priv_protocol.as_str() {
                    "" => None,
                    other => Some(
                        PrivProtocol::parse(other)
                            .ok_or_else(|| invalid("profile-priv-protocol-invalid"))?,
                    ),
                },
                priv_password: secret(&self.priv_password),
            },
        };
        Ok(ProfileInput {
            name: self.name.clone(),
            kind,
        })
    }
}

fn flash(t: Lang, msg: Option<&str>) -> Option<String> {
    let id = match msg? {
        "created" => "flash-profile-created",
        "updated" => "flash-profile-updated",
        "deleted" => "flash-profile-deleted",
        _ => return None,
    };
    Some(t.get(id))
}

pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<ListQuery>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let page = ListTemplate {
        layout: user.layout(),
        t: user.lang,
        profiles: store::list_profiles(&state.pool).await?,
        flash: flash(user.lang, query.msg.as_deref()),
    };
    Ok(Html(page.render()?))
}

pub async fn new_form(user: CurrentUser) -> Result<Html<String>> {
    user.require_admin()?;
    let page = FormTemplate::new(&user, FormValues::default(), None);
    Ok(Html(page.render()?))
}

pub async fn edit_form(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let view = store::get_profile(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)?;
    let page = FormTemplate::new(&user, FormValues::from_view(&view), Some(&view));
    Ok(Html(page.render()?))
}

pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Form(form): Form<ProfileForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    save(&state, &user, None, &form).await
}

pub async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<ProfileForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    save(&state, &user, Some(id), &form).await
}

async fn save(
    state: &AppState,
    user: &CurrentUser,
    id: Option<i64>,
    form: &ProfileForm,
) -> Result<Response> {
    let before = match id {
        Some(id) => store::get_profile(&state.pool, id).await?,
        None => None,
    };
    let outcome = match form.to_input() {
        Ok(input) => store::save_profile(&state.pool, &state.secrets, id, &input)
            .await
            .map(|saved| (saved, input)),
        Err(e) => Err(e),
    };
    match outcome {
        Ok((saved_id, input)) => {
            tracing::info!(
                user = %user.username,
                profile = saved_id,
                action = if id.is_some() { "update" } else { "create" },
                "credential profile saved"
            );
            if let Some(after) = store::get_profile(&state.pool, saved_id).await? {
                audit_saved(state, user, before.as_ref(), &after, &input).await;
            }
            let msg = if id.is_some() { "updated" } else { "created" };
            Ok(Redirect::to(&format!("/credentials?msg={msg}")).into_response())
        }
        Err(Error::Invalid(message) | Error::Conflict(message)) => {
            let existing = match id {
                Some(id) => Some(
                    store::get_profile(&state.pool, id)
                        .await?
                        .ok_or(Error::NotFound)?,
                ),
                None => None,
            };
            let mut page = FormTemplate::new(user, form.values(), existing.as_ref());
            page.error = Some(user.lang.msg(&message));
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(page.render()?)).into_response())
        }
        Err(e) => Err(e),
    }
}

/// The audit entry of a saved profile: settings old → new, secrets only as "changed".
async fn audit_saved(
    state: &AppState,
    user: &CurrentUser,
    before: Option<&ProfileView>,
    after: &ProfileView,
    input: &ProfileInput,
) {
    let settings = |v: &ProfileView| {
        let mut parts = vec![v.version.label().to_owned()];
        parts.extend(v.level.map(|l| l.label().to_owned()));
        parts.extend(v.auth_protocol.map(|p| p.label().to_owned()));
        parts.extend(v.priv_protocol.map(|p| p.label().to_owned()));
        parts.join(" · ")
    };
    let username = |v: &ProfileView| v.username.clone().unwrap_or_default();
    let mut changes = match before {
        Some(old) => Changes::new()
            .field("audit-field-name", &old.name, &after.name)
            .field("audit-field-snmp", settings(old), settings(after))
            .field("audit-field-username", username(old), username(after)),
        None => Changes::new().value("audit-field-snmp", settings(after)),
    };
    let new_secret = |s: &Option<_>| s.is_some();
    changes = match &input.kind {
        ProfileKind::V2c { community } => {
            changes.secret("audit-field-community", new_secret(community))
        }
        ProfileKind::V3 {
            auth_password,
            priv_password,
            ..
        } => changes
            .secret("audit-field-auth-password", new_secret(auth_password))
            .secret("audit-field-priv-password", new_secret(priv_password)),
    };
    let action = if before.is_some() {
        Action::Changed
    } else {
        Action::Created
    };
    audit::record(
        state,
        user,
        action,
        Object::new(Kind::Profile, after.id, &after.name),
        changes,
    )
    .await;
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
        return Err(invalid("confirm-delete-missing"));
    }
    let profile = store::get_profile(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)?;
    if !store::delete_profile(&state.pool, id).await? {
        return Err(Error::NotFound);
    }
    tracing::info!(user = %user.username, profile = id, "credential profile deleted");
    audit::record(
        &state,
        &user,
        Action::Deleted,
        Object::new(Kind::Profile, id, &profile.name),
        Changes::new(),
    )
    .await;
    Ok(Redirect::to("/credentials?msg=deleted").into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::Credentials;
    use crate::web::testutil::*;
    use axum::http::StatusCode;

    const COMMUNITY: &str = "geheime-community-42";
    const AUTH_PW: &str = "auth-passwort-4711";
    const PRIV_PW: &str = "priv-passwort-4711";

    fn enc(value: &str) -> String {
        value.replace('%', "%25").replace(' ', "+")
    }

    fn v3_form(
        csrf: &str,
        name: &str,
        level: &str,
        auth_pw: &str,
        priv_proto: &str,
        priv_pw: &str,
    ) -> String {
        format!(
            "csrf_token={csrf}&name={}&version=v3&community=&username=monitor&level={level}\
             &auth_protocol=md5&auth_password={}&priv_protocol={priv_proto}&priv_password={}",
            enc(name),
            enc(auth_pw),
            enc(priv_pw)
        )
    }

    #[tokio::test]
    async fn anonymous_is_redirected_and_viewer_is_forbidden() {
        let h = harness(true).await;
        for path in ["/credentials", "/credentials/new", "/credentials/1/edit"] {
            let reply = send(&h.app, get(path, None)).await;
            assert_eq!(reply.status, StatusCode::SEE_OTHER, "{path}");
            assert_eq!(reply.location(), "/login");
        }
        let (viewer, csrf) = login(&h, "bert").await;
        for path in ["/credentials", "/credentials/new"] {
            let reply = send(&h.app, get(path, Some(&viewer))).await;
            assert_eq!(reply.status, StatusCode::FORBIDDEN, "{path}");
        }
        let form = v3_form(&csrf, "Omada", "auth_no_priv", AUTH_PW, "", "");
        let reply = send(&h.app, post("/credentials", Some(&viewer), &form)).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        assert!(
            store::list_profiles(&h.state.pool)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn navigation_link_is_only_shown_to_admins() {
        let h = harness(true).await;
        let (admin, _) = login(&h, "anna").await;
        let (viewer, _) = login(&h, "bert").await;
        assert!(
            send(&h.app, get("/", Some(&admin)))
                .await
                .body
                .contains("/credentials")
        );
        assert!(
            !send(&h.app, get("/", Some(&viewer)))
                .await
                .body
                .contains("/credentials")
        );
    }

    #[tokio::test]
    async fn admin_creates_an_omada_profile_and_secrets_never_leak_into_html() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;

        let form = v3_form(&csrf, "Omada Switches", "auth_no_priv", AUTH_PW, "", "");
        let reply = send(&h.app, post("/credentials", Some(&admin), &form)).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        assert_eq!(reply.location(), "/credentials?msg=created");

        let list = send(&h.app, get("/credentials?msg=created", Some(&admin))).await;
        assert_eq!(list.status, StatusCode::OK);
        assert!(list.body.contains("Omada Switches"));
        assert!(
            list.body
                .contains("SNMPv3 · authNoPriv · MD5 · Benutzer monitor")
        );
        assert!(list.body.contains("Profil angelegt."));

        let edit = send(&h.app, get("/credentials/1/edit", Some(&admin))).await;
        assert_eq!(edit.status, StatusCode::OK);
        assert!(edit.body.contains("gesetzt – leer lassen zum Beibehalten"));
        assert!(edit.body.contains("monitor"));
        for page in [&list.body, &edit.body] {
            assert!(!page.contains(AUTH_PW));
        }

        match store::load_credentials(&h.state.pool, &h.state.secrets, 1)
            .await
            .unwrap()
            .unwrap()
        {
            Credentials::V3 {
                auth_password,
                privacy,
                ..
            } => {
                assert_eq!(auth_password.expose(), AUTH_PW);
                assert!(privacy.is_none());
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn editing_with_empty_password_fields_keeps_the_stored_secrets() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let form = v3_form(&csrf, "Core", "auth_priv", AUTH_PW, "aes128", PRIV_PW);
        send(&h.app, post("/credentials", Some(&admin), &form)).await;

        let rename = v3_form(&csrf, "Core umbenannt", "auth_priv", "", "aes128", "");
        let reply = send(&h.app, post("/credentials/1", Some(&admin), &rename)).await;
        assert_eq!(reply.location(), "/credentials?msg=updated");

        match store::load_credentials(&h.state.pool, &h.state.secrets, 1)
            .await
            .unwrap()
            .unwrap()
        {
            Credentials::V3 {
                auth_password,
                privacy,
                ..
            } => {
                assert_eq!(auth_password.expose(), AUTH_PW);
                assert_eq!(privacy.unwrap().1.expose(), PRIV_PW);
            }
            other => panic!("unexpected {other:?}"),
        }
        let edit = send(&h.app, get("/credentials/1/edit", Some(&admin))).await;
        assert!(edit.body.contains("Core umbenannt"));
        assert!(!edit.body.contains(AUTH_PW) && !edit.body.contains(PRIV_PW));
    }

    #[tokio::test]
    async fn v2c_profile_roundtrip_hides_the_community() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let form = format!(
            "csrf_token={csrf}&name=Alt&version=v2c&community={COMMUNITY}&username=&level=&auth_protocol=&priv_protocol="
        );
        let reply = send(&h.app, post("/credentials", Some(&admin), &form)).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        let edit = send(&h.app, get("/credentials/1/edit", Some(&admin))).await;
        assert!(!edit.body.contains(COMMUNITY));
        assert!(edit.body.contains("gesetzt – leer lassen zum Beibehalten"));
    }

    #[tokio::test]
    async fn validation_errors_rerender_the_form_without_echoing_passwords() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;

        // Auth password too short.
        let form = v3_form(&csrf, "Zu kurz", "auth_no_priv", "kurz", "", "");
        let reply = send(&h.app, post("/credentials", Some(&admin), &form)).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(reply.body.contains("8 bis 128 Zeichen"));
        assert!(reply.body.contains("Zu kurz"));
        assert!(!reply.body.contains("kurz</"), "password echoed");

        // authPriv without a privacy protocol.
        let form = v3_form(&csrf, "Ohne Priv", "auth_priv", AUTH_PW, "", PRIV_PW);
        let reply = send(&h.app, post("/credentials", Some(&admin), &form)).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!reply.body.contains(AUTH_PW) && !reply.body.contains(PRIV_PW));

        // Missing password on create.
        let form = v3_form(&csrf, "Ohne Passwort", "auth_no_priv", "", "", "");
        let reply = send(&h.app, post("/credentials", Some(&admin), &form)).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(reply.body.contains("Authentifizierungs-Passwort"));

        // Unknown protocol value.
        let form = v3_form(&csrf, "Kaputt", "auth_no_priv", AUTH_PW, "", "")
            .replace("auth_protocol=md5", "auth_protocol=rot13");
        let reply = send(&h.app, post("/credentials", Some(&admin), &form)).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);

        assert!(
            store::list_profiles(&h.state.pool)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn duplicate_names_show_a_message() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let form = v3_form(&csrf, "Omada", "auth_no_priv", AUTH_PW, "", "");
        send(&h.app, post("/credentials", Some(&admin), &form)).await;
        let reply = send(&h.app, post("/credentials", Some(&admin), &form)).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(reply.body.contains("existiert bereits"));
    }

    #[tokio::test]
    async fn posts_without_a_valid_csrf_token_are_rejected() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let form = v3_form("falsch", "Omada", "auth_no_priv", AUTH_PW, "", "");
        let reply = send(&h.app, post("/credentials", Some(&admin), &form)).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);

        let form = v3_form(&csrf, "Omada", "auth_no_priv", AUTH_PW, "", "");
        send(&h.app, post("/credentials", Some(&admin), &form)).await;
        let reply = send(
            &h.app,
            post(
                "/credentials/1/delete",
                Some(&admin),
                "csrf_token=falsch&confirm=yes",
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        assert_eq!(store::list_profiles(&h.state.pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn delete_requires_confirmation_and_removes_the_profile() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let form = v3_form(&csrf, "Omada", "auth_no_priv", AUTH_PW, "", "");
        send(&h.app, post("/credentials", Some(&admin), &form)).await;

        let reply = send(
            &h.app,
            post(
                "/credentials/1/delete",
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(store::list_profiles(&h.state.pool).await.unwrap().len(), 1);

        let reply = send(
            &h.app,
            post(
                "/credentials/1/delete",
                Some(&admin),
                &format!("csrf_token={csrf}&confirm=yes"),
            ),
        )
        .await;
        assert_eq!(reply.location(), "/credentials?msg=deleted");
        assert!(
            store::list_profiles(&h.state.pool)
                .await
                .unwrap()
                .is_empty()
        );

        let reply = send(
            &h.app,
            post(
                "/credentials/1/delete",
                Some(&admin),
                &format!("csrf_token={csrf}&confirm=yes"),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn unknown_profile_is_404_and_unknown_flash_is_ignored() {
        let h = harness(true).await;
        let (admin, _) = login(&h, "anna").await;
        assert_eq!(
            send(&h.app, get("/credentials/99/edit", Some(&admin)))
                .await
                .status,
            StatusCode::NOT_FOUND
        );
        let list = send(&h.app, get("/credentials?msg=%3Cscript%3E", Some(&admin))).await;
        assert_eq!(list.status, StatusCode::OK);
        assert!(!list.body.contains("<script>"));
    }

    #[tokio::test]
    async fn user_supplied_text_is_html_escaped() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let form = v3_form(&csrf, "<b>x</b>", "auth_no_priv", AUTH_PW, "", "");
        send(&h.app, post("/credentials", Some(&admin), &form)).await;
        let list = send(&h.app, get("/credentials", Some(&admin))).await;
        assert!(!list.body.contains("<b>x</b>"));
        assert!(
            list.body.contains("&#60;b&#62;x&#60;/b&#62;")
                || list.body.contains("&lt;b&gt;x&lt;/b&gt;")
        );
    }

    #[tokio::test]
    async fn the_pages_and_their_errors_are_shown_in_english() {
        let h = harness(true).await;
        crate::storage::users::set_language(&h.state.pool, 1, Some(Lang::En))
            .await
            .unwrap();
        let (admin, csrf) = login(&h, "anna").await;
        let form = send(&h.app, get("/credentials/new", Some(&admin))).await;
        assert!(form.body.contains("<h1>New profile</h1>"));
        assert!(form.body.contains("authNoPriv (authentication only)"));
        let refused = send(
            &h.app,
            post(
                "/credentials",
                Some(&admin),
                &format!("csrf_token={csrf}&name=Lab&version=v2c&community="),
            ),
        )
        .await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(refused.body.contains("Please enter a community."));
        let list = send(&h.app, get("/credentials", Some(&admin))).await;
        assert!(list.body.contains("No profiles yet."));
    }
}
