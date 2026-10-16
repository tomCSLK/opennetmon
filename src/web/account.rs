//! "Mein Konto" for every signed-in user: shows name and role and changes the own password
//! (current password required; wrong attempts count towards the login throttle).

use askama::Template;
use axum::Form;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::auth;
use crate::error::{Error, Result};
use crate::storage::users as store;
use crate::web::auth::{CurrentUser, SESSION_COOKIE, cookie_value};
use crate::web::{AppState, Layout, flash_message};

#[derive(Template)]
#[template(path = "account.html")]
struct AccountTemplate {
    layout: Layout,
    role_label: &'static str,
    flash: Option<&'static str>,
    error: Option<String>,
}

#[derive(Deserialize)]
pub struct FlashQuery {
    msg: Option<String>,
}

#[derive(Deserialize)]
pub struct PasswordForm {
    csrf_token: String,
    current: String,
    password: String,
    password_repeat: String,
}

fn page(user: &CurrentUser, flash: Option<&'static str>, error: Option<String>) -> Result<String> {
    Ok(AccountTemplate {
        layout: user.layout(),
        role_label: user.role.label(),
        flash,
        error,
    }
    .render()?)
}

pub async fn show(user: CurrentUser, Query(query): Query<FlashQuery>) -> Result<Html<String>> {
    Ok(Html(page(
        &user,
        flash_message(query.msg.as_deref()),
        None,
    )?))
}

pub async fn change_password(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Form(form): Form<PasswordForm>,
) -> Result<Response> {
    user.verify_csrf(&form.csrf_token)?;
    let refuse = |message: String| -> Result<Response> {
        Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Html(page(&user, None, Some(message))?),
        )
            .into_response())
    };
    let now_ms = auth::now_secs().saturating_mul(1000);
    if state
        .login_throttle
        .check(now_ms, &user.username, None)
        .is_err()
    {
        return refuse("Zu viele falsche Passwörter. Bitte später erneut versuchen.".to_owned());
    }
    let stored = store::get_user(&state.pool, user.id)
        .await?
        .ok_or(Error::NotFound)?;
    let current = form.current.clone();
    let verified =
        tokio::task::spawn_blocking(move || auth::verify_password(&stored.password_hash, &current))
            .await
            .map_err(|e| Error::Internal(format!("password check task failed: {e}")))?;
    if !verified {
        state
            .login_throttle
            .record_failure(now_ms, &user.username, None);
        tracing::info!(user = %user.username, "own password change refused: wrong current password");
        return refuse("Das bisherige Passwort ist falsch.".to_owned());
    }
    if let Err(message) = auth::validate_password(&form.password) {
        return refuse(message.to_owned());
    }
    if form.password != form.password_repeat {
        return refuse("Die beiden neuen Passwörter stimmen nicht überein.".to_owned());
    }
    if form.password == form.current {
        return refuse("Das neue Passwort muss sich vom bisherigen unterscheiden.".to_owned());
    }
    let hash = auth::hash_password(&form.password)?;
    let keep = cookie_value(&headers, SESSION_COOKIE).map(|t| auth::token_hash(&t));
    store::change_password(&state.pool, user.id, &hash, keep.as_deref()).await?;
    state.login_throttle.record_success(&user.username, None);
    tracing::info!(user = %user.username, "own password changed");
    Ok(Redirect::to("/account?msg=account_password_saved").into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::testutil::*;

    const NEW_PW: &str = "mein-neues-passwort";

    fn form(csrf: &str, current: &str, new: &str, repeat: &str) -> String {
        format!("csrf_token={csrf}&current={current}&password={new}&password_repeat={repeat}")
    }

    #[tokio::test]
    async fn everyone_changes_their_own_password_with_the_current_one() {
        let h = harness(true).await;
        let (viewer, csrf) = login(&h, "bert").await;
        let (elsewhere, _) = login(&h, "bert").await;
        let page = send(&h.app, get("/account", Some(&viewer))).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(
            page.body.contains("Mein Konto")
                && page.body.contains("Leser")
                && page.body.contains("bert")
        );

        for (body, text) in [
            (
                form(&csrf, "falsch-falsch", NEW_PW, NEW_PW),
                "bisherige Passwort ist falsch",
            ),
            (
                form(&csrf, PASSWORD, "kurz", "kurz"),
                "mindestens 10 Zeichen",
            ),
            (
                form(&csrf, PASSWORD, NEW_PW, "etwas-anderes"),
                "stimmen nicht überein",
            ),
            (form(&csrf, PASSWORD, PASSWORD, PASSWORD), "unterscheiden"),
        ] {
            let reply = send(&h.app, post("/account/password", Some(&viewer), &body)).await;
            assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{text}");
            assert!(reply.body.contains(text), "{text}");
            assert!(!reply.body.contains(PASSWORD) && !reply.body.contains(NEW_PW));
        }

        let reply = send(
            &h.app,
            post(
                "/account/password",
                Some(&viewer),
                &form(&csrf, PASSWORD, NEW_PW, NEW_PW),
            ),
        )
        .await;
        assert_eq!(reply.location(), "/account?msg=account_password_saved");
        assert_eq!(
            send(&h.app, get("/", Some(&viewer))).await.status,
            StatusCode::OK,
            "this session stays"
        );
        assert_eq!(
            send(&h.app, get("/", Some(&elsewhere))).await.status,
            StatusCode::SEE_OTHER,
            "others end"
        );
        let stored = store::find_by_username(&h.state.pool, "bert")
            .await
            .unwrap()
            .unwrap();
        assert!(auth::verify_password(&stored.password_hash, NEW_PW));
        assert_eq!(
            send(&h.app, get("/account", None)).await.status,
            StatusCode::SEE_OTHER
        );
    }

    #[tokio::test]
    async fn guessing_the_current_password_is_throttled() {
        let h = harness(true).await;
        let (viewer, csrf) = login(&h, "bert").await;
        for _ in 0..6 {
            send(
                &h.app,
                post(
                    "/account/password",
                    Some(&viewer),
                    &form(&csrf, "geraten-123", NEW_PW, NEW_PW),
                ),
            )
            .await;
        }
        let reply = send(
            &h.app,
            post(
                "/account/password",
                Some(&viewer),
                &form(&csrf, PASSWORD, NEW_PW, NEW_PW),
            ),
        )
        .await;
        assert!(
            reply.body.contains("Zu viele falsche Passwörter"),
            "{}",
            reply.body
        );
        let stored = store::find_by_username(&h.state.pool, "bert")
            .await
            .unwrap()
            .unwrap();
        assert!(
            auth::verify_password(&stored.password_hash, PASSWORD),
            "unchanged"
        );
    }
}
