//! "Mein Konto" for every signed-in user: shows name and role, chooses the language and changes the own
//! password (current password required; wrong attempts count towards the login throttle).

use askama::Template;
use axum::Form;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::audit::{Action, Changes};
use crate::auth;
use crate::error::{Error, Result};
use crate::i18n::Lang;
use crate::storage::users as store;
use crate::web::auth::{CurrentUser, SESSION_COOKIE, cookie_value};
use crate::web::users::{audit_user, language_change};
use crate::web::{
    AppState, LanguageOption, Layout, flash_message, language_options, parse_language_choice,
};

#[derive(Template)]
#[template(path = "account.html")]
struct AccountTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    languages: Vec<LanguageOption>,
    flash: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
pub struct FlashQuery {
    msg: Option<String>,
}

#[derive(Deserialize)]
pub struct LanguageForm {
    csrf_token: String,
    #[serde(default)]
    language: String,
}

#[derive(Deserialize)]
pub struct PasswordForm {
    csrf_token: String,
    current: String,
    password: String,
    password_repeat: String,
}

fn page(user: &CurrentUser, flash: Option<String>, error: Option<String>) -> Result<String> {
    Ok(AccountTemplate {
        layout: user.layout(),
        t: user.lang,
        languages: language_options(user.lang, user.language),
        flash,
        error,
    }
    .render()?)
}

pub async fn show(user: CurrentUser, Query(query): Query<FlashQuery>) -> Result<Html<String>> {
    Ok(Html(page(
        &user,
        flash_message(user.lang, query.msg.as_deref()),
        None,
    )?))
}

pub async fn set_language(
    State(state): State<AppState>,
    user: CurrentUser,
    Form(form): Form<LanguageForm>,
) -> Result<Response> {
    user.verify_csrf(&form.csrf_token)?;
    match parse_language_choice(&form.language) {
        Ok(language) => {
            store::set_language(&state.pool, user.id, language).await?;
            let changes = language_change(user.language, language);
            audit_user(
                &state,
                &user,
                Action::Changed,
                user.id,
                &user.username,
                changes,
            )
            .await;
            Ok(Redirect::to("/account?msg=language_saved").into_response())
        }
        Err(Error::Invalid(message)) => Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Html(page(&user, None, Some(user.lang.msg(&message)))?),
        )
            .into_response()),
        Err(e) => Err(e),
    }
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
    // Counts as failed until verified, like a sign-in (`LoginThrottle::begin_attempt`).
    if state
        .login_throttle
        .begin_attempt(now_ms, &user.username, None)
        .is_err()
    {
        return refuse(user.lang.get("account-too-many-attempts"));
    }
    let stored = store::get_user(&state.pool, user.id)
        .await?
        .ok_or(Error::NotFound)?;
    let Some(verified) = state
        .check_password(stored.password_hash, form.current.clone())
        .await?
    else {
        return refuse(user.lang.get("login-busy"));
    };
    if !verified {
        tracing::info!(user = %user.username, "own password change refused: wrong current password");
        return refuse(user.lang.get("account-current-wrong"));
    }
    state.login_throttle.record_success(&user.username, None);
    if let Err(message) = auth::validate_password(&form.password) {
        return refuse(user.lang.msg(&message));
    }
    if form.password != form.password_repeat {
        return refuse(user.lang.get("password-mismatch"));
    }
    if form.password == form.current {
        return refuse(user.lang.get("account-password-unchanged"));
    }
    let hash = auth::hash_password(&form.password)?;
    let keep = cookie_value(&headers, SESSION_COOKIE).map(|t| auth::token_hash(&t));
    store::change_password(&state.pool, user.id, &hash, keep.as_deref()).await?;
    tracing::info!(user = %user.username, "own password changed");
    let changes = Changes::new().secret("audit-field-password", true);
    audit_user(
        &state,
        &user,
        Action::Changed,
        user.id,
        &user.username,
        changes,
    )
    .await;
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

    #[tokio::test]
    async fn the_language_follows_the_browser_until_one_is_chosen() {
        let h = harness(true).await;
        let (viewer, csrf) = login(&h, "bert").await;
        let english = send(
            &h.app,
            accepting(get("/account", Some(&viewer)), "en-US,en;q=0.9"),
        )
        .await;
        assert!(english.body.contains("<html lang=\"en\">") && english.body.contains("My account"));
        assert!(
            english
                .body
                .contains("<option value=\"\" selected>Same as the browser</option>")
        );

        let saved = send(
            &h.app,
            post(
                "/account/language",
                Some(&viewer),
                &format!("csrf_token={csrf}&language=de"),
            ),
        )
        .await;
        assert_eq!(saved.status, StatusCode::SEE_OTHER);
        assert_eq!(saved.location(), "/account?msg=language_saved");
        let german = send(
            &h.app,
            accepting(get("/account?msg=language_saved", Some(&viewer)), "en"),
        )
        .await;
        assert!(
            german.body.contains("<html lang=\"de\">")
                && german.body.contains("Sprache gespeichert.")
        );
        assert!(
            german
                .body
                .contains("<option value=\"de\" selected>Deutsch</option>")
        );

        let back = format!("csrf_token={csrf}&language=");
        send(&h.app, post("/account/language", Some(&viewer), &back)).await;
        let page = send(&h.app, accepting(get("/account", Some(&viewer)), "en")).await;
        assert!(page.body.contains("My account"));
    }

    #[tokio::test]
    async fn an_unknown_language_or_a_missing_csrf_token_is_refused() {
        let h = harness(true).await;
        let (viewer, csrf) = login(&h, "bert").await;
        let unknown = send(
            &h.app,
            post(
                "/account/language",
                Some(&viewer),
                &format!("csrf_token={csrf}&language=xx"),
            ),
        )
        .await;
        assert_eq!(unknown.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(unknown.body.contains("Diese Sprache gibt es nicht."));
        let forged = send(
            &h.app,
            post(
                "/account/language",
                Some(&viewer),
                "csrf_token=wrong&language=en",
            ),
        )
        .await;
        assert_eq!(forged.status, StatusCode::FORBIDDEN);
        let stored = store::get_user(&h.state.pool, 2).await.unwrap().unwrap();
        assert_eq!(stored.language, None);
    }
}
