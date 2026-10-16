//! User administration (admin only): list and create users, change role and password, delete.
//! The last administrator can neither be demoted nor deleted, and nobody deletes their own account.

use askama::Template;
use axum::Form;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::auth::{self, Role};
use crate::error::{Error, Result};
use crate::storage::users::{self as store, User, UserInfo};
use crate::web::auth::{CurrentUser, SESSION_COOKIE, cookie_value};
use crate::web::{AppState, Layout, flash_message, iso_time_html};

struct Row {
    info: UserInfo,
    role_label: &'static str,
    created: String,
    is_me: bool,
}

#[derive(Template)]
#[template(path = "users.html")]
struct ListTemplate {
    layout: Layout,
    rows: Vec<Row>,
    flash: Option<&'static str>,
    error: Option<String>,
    /// Entered values of the "new user" form after an error (never the password).
    username: String,
    role: &'static str,
}

#[derive(Template)]
#[template(path = "user_edit.html")]
struct EditTemplate {
    layout: Layout,
    user: User,
    role_label: &'static str,
    is_me: bool,
    flash: Option<&'static str>,
    error: Option<String>,
}

#[derive(Deserialize)]
pub struct FlashQuery {
    msg: Option<String>,
}

#[derive(Deserialize)]
pub struct NewUserForm {
    csrf_token: String,
    username: String,
    #[serde(default)]
    role: String,
    password: String,
    password_repeat: String,
}

#[derive(Deserialize)]
pub struct RoleForm {
    csrf_token: String,
    role: String,
}

#[derive(Deserialize)]
pub struct PasswordForm {
    csrf_token: String,
    password: String,
    password_repeat: String,
}

#[derive(Deserialize)]
pub struct DeleteForm {
    csrf_token: String,
    confirm: Option<String>,
}

fn parse_role(text: &str) -> Result<Role> {
    Role::parse(text).ok_or_else(|| Error::Invalid("Bitte eine Rolle wählen.".to_owned()))
}

fn checked_password(password: &str, repeat: &str) -> Result<String> {
    auth::validate_password(password).map_err(|m| Error::Invalid(m.to_owned()))?;
    if password != repeat {
        return Err(Error::Invalid(
            "Die beiden Passwörter stimmen nicht überein.".to_owned(),
        ));
    }
    auth::hash_password(password)
}

async fn render_list(
    state: &AppState,
    user: &CurrentUser,
    flash: Option<&'static str>,
    error: Option<String>,
    username: String,
    role: Role,
) -> Result<String> {
    let rows = store::list_user_infos(&state.pool, auth::now_secs())
        .await?
        .into_iter()
        .map(|info| Row {
            role_label: Role::parse(&info.role).map_or("?", Role::label),
            created: iso_time_html(&info.created_at),
            is_me: info.id == user.id,
            info,
        })
        .collect();
    let page = ListTemplate {
        layout: user.layout(),
        rows,
        flash,
        error,
        username,
        role: role.as_str(),
    };
    Ok(page.render()?)
}

pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<FlashQuery>,
) -> Result<Html<String>> {
    user.require_admin()?;
    Ok(Html(
        render_list(
            &state,
            &user,
            flash_message(query.msg.as_deref()),
            None,
            String::new(),
            Role::Viewer,
        )
        .await?,
    ))
}

pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Form(form): Form<NewUserForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let username = form.username.trim().to_owned();
    let role = Role::parse(&form.role).unwrap_or(Role::Viewer);
    let outcome = async {
        auth::validate_username(&username).map_err(|m| Error::Invalid(m.to_owned()))?;
        parse_role(&form.role)?;
        let hash = checked_password(&form.password, &form.password_repeat)?;
        store::create_user(&state.pool, &username, &hash, role).await
    }
    .await;
    match outcome {
        Ok(id) => {
            tracing::info!(user = %user.username, created = id, role = role.as_str(), "user created");
            Ok(Redirect::to("/users?msg=user_created").into_response())
        }
        Err(Error::Invalid(message) | Error::Conflict(message)) => {
            let html = render_list(&state, &user, None, Some(message), username, role).await?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response())
        }
        Err(e) => Err(e),
    }
}

async fn load(state: &AppState, id: i64) -> Result<User> {
    store::get_user(&state.pool, id)
        .await?
        .ok_or(Error::NotFound)
}

fn edit_page(
    user: &CurrentUser,
    target: User,
    flash: Option<&'static str>,
    error: Option<String>,
) -> Result<String> {
    let page = EditTemplate {
        layout: user.layout(),
        role_label: target.role.label(),
        is_me: target.id == user.id,
        user: target,
        flash,
        error,
    };
    Ok(page.render()?)
}

pub async fn edit(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<FlashQuery>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let target = load(&state, id).await?;
    Ok(Html(edit_page(
        &user,
        target,
        flash_message(query.msg.as_deref()),
        None,
    )?))
}

/// Renders the edit page with a user-facing error, or passes other errors on.
async fn refused(state: &AppState, user: &CurrentUser, id: i64, error: Error) -> Result<Response> {
    match error {
        Error::Invalid(message) | Error::Conflict(message) => {
            let target = load(state, id).await?;
            let html = edit_page(user, target, None, Some(message))?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response())
        }
        e => Err(e),
    }
}

pub async fn set_role(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<RoleForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    load(&state, id).await?;
    let outcome = match parse_role(&form.role) {
        Ok(role) => store::set_role(&state.pool, id, role).await.map(|()| role),
        Err(e) => Err(e),
    };
    match outcome {
        Ok(role) => {
            tracing::info!(user = %user.username, target = id, role = role.as_str(), "user role changed");
            // An admin who made themselves a viewer can no longer open this page.
            let to = if id == user.id && !role.is_admin() {
                "/?msg=user_role_saved".to_owned()
            } else {
                format!("/users/{id}?msg=user_role_saved")
            };
            Ok(Redirect::to(&to).into_response())
        }
        Err(e) => refused(&state, &user, id, e).await,
    }
}

pub async fn set_password(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Form(form): Form<PasswordForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    load(&state, id).await?;
    // Changing one's own password keeps this session; every other session of the user ends.
    let keep = (id == user.id)
        .then(|| cookie_value(&headers, SESSION_COOKIE).map(|t| auth::token_hash(&t)))
        .flatten();
    let outcome = match checked_password(&form.password, &form.password_repeat) {
        Ok(hash) => store::change_password(&state.pool, id, &hash, keep.as_deref()).await,
        Err(e) => Err(e),
    };
    match outcome {
        Ok(()) => {
            tracing::info!(user = %user.username, target = id, "user password changed");
            Ok(Redirect::to(&format!("/users/{id}?msg=user_password_saved")).into_response())
        }
        Err(e) => refused(&state, &user, id, e).await,
    }
}

pub async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<DeleteForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let target = load(&state, id).await?;
    let outcome = if id == user.id {
        Err(Error::Invalid(
            "Das eigene Konto lässt sich nicht löschen; das muss ein anderer Administrator tun."
                .to_owned(),
        ))
    } else if form.confirm.as_deref() != Some("yes") {
        Err(Error::Invalid("Bitte das Löschen bestätigen.".to_owned()))
    } else {
        store::delete_user(&state.pool, id).await
    };
    match outcome {
        Ok(()) => {
            tracing::info!(user = %user.username, deleted = %target.username, "user deleted");
            Ok(Redirect::to("/users?msg=user_deleted").into_response())
        }
        Err(e) => refused(&state, &user, id, e).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::testutil::*;

    const NEW_PW: &str = "ein-langes-passwort";

    fn new_user(csrf: &str, name: &str, role: &str, pw: &str, repeat: &str) -> String {
        format!(
            "csrf_token={csrf}&username={name}&role={role}&password={pw}&password_repeat={repeat}"
        )
    }

    async fn id_of(h: &Harness, name: &str) -> i64 {
        store::find_by_username(&h.state.pool, name)
            .await
            .unwrap()
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn admins_create_users_who_can_sign_in() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(&h.app, get("/users", Some(&admin))).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(
            page.body.contains("anna") && page.body.contains("(du)") && page.body.contains("bert")
        );

        let reply = send(
            &h.app,
            post(
                "/users",
                Some(&admin),
                &new_user(&csrf, "lena", "viewer", NEW_PW, NEW_PW),
            ),
        )
        .await;
        assert_eq!(reply.location(), "/users?msg=user_created");
        let lena = store::find_by_username(&h.state.pool, "lena")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lena.role, Role::Viewer);
        assert!(auth::verify_password(&lena.password_hash, NEW_PW));

        for (body, text) in [
            (
                new_user(&csrf, "LENA", "viewer", NEW_PW, NEW_PW),
                "bereits vergeben",
            ),
            (
                new_user(&csrf, "x", "viewer", NEW_PW, NEW_PW),
                "3 bis 32 Zeichen",
            ),
            (
                new_user(&csrf, "otto", "admin", "kurz", "kurz"),
                "mindestens 10 Zeichen",
            ),
            (
                new_user(&csrf, "otto", "admin", NEW_PW, "anders-als-oben"),
                "stimmen nicht überein",
            ),
            (new_user(&csrf, "otto", "root", NEW_PW, NEW_PW), "Rolle"),
        ] {
            let page = send(&h.app, post("/users", Some(&admin), &body)).await;
            assert_eq!(page.status, StatusCode::UNPROCESSABLE_ENTITY, "{text}");
            assert!(page.body.contains(text), "{text}: {}", page.body);
            assert!(!page.body.contains(NEW_PW), "passwords are never echoed");
        }
        let page = send(
            &h.app,
            post(
                "/users",
                Some(&admin),
                &new_user(&csrf, "otto", "admin", "kurz", "kurz"),
            ),
        )
        .await;
        assert!(
            page.body.contains("value=\"otto\"")
                && page.body.contains("<option value=\"admin\" selected>")
        );
        assert!(
            store::find_by_username(&h.state.pool, "otto")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn roles_change_at_once_but_the_last_admin_stays() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let (viewer, _) = login(&h, "bert").await;
        let (anna, bert) = (id_of(&h, "anna").await, id_of(&h, "bert").await);

        let refused = send(
            &h.app,
            post(
                &format!("/users/{anna}/role"),
                Some(&admin),
                &format!("csrf_token={csrf}&role=viewer"),
            ),
        )
        .await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(refused.body.contains("letzte Administrator"));

        assert_eq!(
            send(&h.app, get("/users", Some(&viewer))).await.status,
            StatusCode::FORBIDDEN
        );
        let reply = send(
            &h.app,
            post(
                &format!("/users/{bert}/role"),
                Some(&admin),
                &format!("csrf_token={csrf}&role=admin"),
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/users/{bert}?msg=user_role_saved")
        );
        assert_eq!(
            send(&h.app, get("/users", Some(&viewer))).await.status,
            StatusCode::OK,
            "effective in the open session"
        );

        // Now anna may step down; she lands on the overview.
        let reply = send(
            &h.app,
            post(
                &format!("/users/{anna}/role"),
                Some(&admin),
                &format!("csrf_token={csrf}&role=viewer"),
            ),
        )
        .await;
        assert_eq!(reply.location(), "/?msg=user_role_saved");
        assert_eq!(
            send(&h.app, get("/users", Some(&admin))).await.status,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn passwords_end_other_sessions_and_deleting_is_guarded() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let (admin_elsewhere, _) = login(&h, "anna").await;
        let (viewer, vcsrf) = login(&h, "bert").await;
        let (anna, bert) = (id_of(&h, "anna").await, id_of(&h, "bert").await);
        let password = |pw: &str| format!("csrf_token={csrf}&password={pw}&password_repeat={pw}");

        // Own password: this session stays, the other one ends.
        let reply = send(
            &h.app,
            post(
                &format!("/users/{anna}/password"),
                Some(&admin),
                &password(NEW_PW),
            ),
        )
        .await;
        assert_eq!(
            reply.location(),
            format!("/users/{anna}?msg=user_password_saved")
        );
        assert_eq!(
            send(&h.app, get("/", Some(&admin))).await.status,
            StatusCode::OK
        );
        assert_eq!(
            send(&h.app, get("/", Some(&admin_elsewhere))).await.status,
            StatusCode::SEE_OTHER
        );

        // Someone else's password: all their sessions end.
        send(
            &h.app,
            post(
                &format!("/users/{bert}/password"),
                Some(&admin),
                &password(NEW_PW),
            ),
        )
        .await;
        assert_eq!(
            send(&h.app, get("/", Some(&viewer))).await.status,
            StatusCode::SEE_OTHER
        );
        let bad = send(
            &h.app,
            post(
                &format!("/users/{bert}/password"),
                Some(&admin),
                &format!("csrf_token={csrf}&password={NEW_PW}&password_repeat=x"),
            ),
        )
        .await;
        assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);

        // Deleting: not oneself, only with confirmation, viewers never.
        let own = send(
            &h.app,
            post(
                &format!("/users/{anna}/delete"),
                Some(&admin),
                &format!("csrf_token={csrf}&confirm=yes"),
            ),
        )
        .await;
        assert!(own.body.contains("eigene Konto"));
        let unconfirmed = send(
            &h.app,
            post(
                &format!("/users/{bert}/delete"),
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(unconfirmed.status, StatusCode::UNPROCESSABLE_ENTITY);
        let page = send(&h.app, get(&format!("/users/{anna}"), Some(&admin))).await;
        assert!(!page.body.contains("/delete"), "no delete form for oneself");
        assert_eq!(
            send(
                &h.app,
                post(
                    &format!("/users/{anna}/delete"),
                    Some(&viewer),
                    &format!("csrf_token={vcsrf}&confirm=yes")
                )
            )
            .await
            .status,
            StatusCode::SEE_OTHER,
            "signed out viewers go to the login page"
        );
        let reply = send(
            &h.app,
            post(
                &format!("/users/{bert}/delete"),
                Some(&admin),
                &format!("csrf_token={csrf}&confirm=yes"),
            ),
        )
        .await;
        assert_eq!(reply.location(), "/users?msg=user_deleted");
        assert!(
            store::get_user(&h.state.pool, bert)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            send(&h.app, get("/users/999", Some(&admin))).await.status,
            StatusCode::NOT_FOUND
        );
    }
}
