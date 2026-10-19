//! Login, logout, session cookies and the `CurrentUser` extractor.

use askama::Template;
use axum::Form;
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{AppendHeaders, Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::auth::{self, Role, throttle};
use crate::error::{Error, Result};
use crate::storage::users::{self, SessionUser};
use crate::web::{AppState, Layout};

pub(crate) const SESSION_COOKIE: &str = "opennetmon_session";
pub(crate) const LOGIN_CSRF_COOKIE: &str = "opennetmon_login";
const LOGIN_CSRF_TTL_SECS: i64 = 3600;
/// Bounds for credentials in a login request; keeps Argon2 work bounded.
const MAX_LOGIN_USERNAME_LEN: usize = 64;
const MAX_LOGIN_PASSWORD_LEN: usize = 1024;

/// The authenticated user of the current request. Unauthenticated requests are redirected to `/login`.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub id: i64,
    pub username: String,
    pub role: Role,
    pub csrf_token: String,
}

impl CurrentUser {
    /// For admin-only handlers.
    pub fn require_admin(&self) -> Result<()> {
        if self.role.is_admin() {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }

    /// Verifies the CSRF token submitted with a POST form.
    pub fn verify_csrf(&self, submitted: &str) -> Result<()> {
        if auth::secrets_match(&self.csrf_token, submitted) {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }
}

impl CurrentUser {
    /// Header data for page templates.
    pub fn layout(&self) -> Layout {
        Layout {
            username: self.username.clone(),
            role_label: self.role.label(),
            csrf_token: self.csrf_token.clone(),
            is_admin: self.role.is_admin(),
        }
    }
}

impl From<SessionUser> for CurrentUser {
    fn from(s: SessionUser) -> Self {
        Self {
            id: s.user_id,
            username: s.username,
            role: s.role,
            csrf_token: s.csrf_token,
        }
    }
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> std::result::Result<Self, Self::Rejection> {
        let Some(token) = cookie_value(&parts.headers, SESSION_COOKIE) else {
            return Err(Redirect::to("/login").into_response());
        };
        match users::find_session(&state.pool, &auth::token_hash(&token), auth::now_secs()).await {
            Ok(Some(session)) => Ok(session.into()),
            Ok(None) => Err(Redirect::to("/login").into_response()),
            Err(e) => Err(e.into_response()),
        }
    }
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate<'a> {
    csrf_token: &'a str,
    error: Option<&'a str>,
}

/// Deliberately has no `Debug`: the password must never end up in logs.
#[derive(Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
    csrf_token: String,
}

#[derive(Deserialize)]
pub struct CsrfForm {
    pub(crate) csrf_token: String,
}

pub async fn login_form(State(state): State<AppState>) -> Result<Response> {
    let csrf = auth::random_token();
    let body = LoginTemplate {
        csrf_token: &csrf,
        error: None,
    }
    .render()?;
    let cookie = set_cookie(
        LOGIN_CSRF_COOKIE,
        &csrf,
        LOGIN_CSRF_TTL_SECS,
        state.cookie_secure,
    );
    Ok((AppendHeaders([(header::SET_COOKIE, cookie)]), Html(body)).into_response())
}

/// Human-readable waiting time, e.g. `20 Sekunden` or `3 Minuten`.
fn wait_text(ms: i64) -> String {
    let secs = (ms.max(0) + 999) / 1000;
    let (n, one, many) = if secs < 60 {
        (secs, "Sekunde", "Sekunden")
    } else {
        ((secs + 59) / 60, "Minute", "Minuten")
    };
    format!("{n} {}", if n == 1 { one } else { many })
}

pub async fn login_submit(
    State(state): State<AppState>,
    extensions: axum::http::Extensions,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Result<Response> {
    // Double-submit CSRF protection for the pre-session form.
    let cookie_csrf = cookie_value(&headers, LOGIN_CSRF_COOKIE).unwrap_or_default();
    if !auth::secrets_match(&cookie_csrf, &form.csrf_token) {
        return Err(Error::Forbidden);
    }

    // Refuse early (without touching the password) while this user name or address is being throttled.
    let ip = throttle::client_ip(
        extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip()),
        &headers,
        state.client_ip_header,
    );
    let now_ms = auth::now_secs().saturating_mul(1000);
    if let Err(wait_ms) = state.login_throttle.check(now_ms, &form.username, ip) {
        tracing::warn!(?ip, wait_ms, "login attempt refused (too many failures)");
        let body = LoginTemplate {
            csrf_token: &cookie_csrf,
            error: Some(&format!(
                "Zu viele fehlgeschlagene Anmeldeversuche. Bitte in {} erneut versuchen.",
                wait_text(wait_ms)
            )),
        }
        .render()?;
        let retry_after = ((wait_ms + 999) / 1000).max(1).to_string();
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            AppendHeaders([(header::RETRY_AFTER, retry_after)]),
            Html(body),
        )
            .into_response());
    }

    let user = if form.username.chars().count() <= MAX_LOGIN_USERNAME_LEN
        && form.password.chars().count() <= MAX_LOGIN_PASSWORD_LEN
    {
        users::find_by_username(&state.pool, &form.username).await?
    } else {
        None
    };

    // Always run one Argon2 verification so unknown users cost the same as wrong passwords.
    let hash = user.as_ref().map_or_else(
        || auth::dummy_hash().to_owned(),
        |u| u.password_hash.clone(),
    );
    let password = form.password;
    let verified = tokio::task::spawn_blocking(move || auth::verify_password(&hash, &password))
        .await
        .map_err(|e| Error::Internal(format!("password check task failed: {e}")))?;

    let user = match user {
        Some(user) if verified => user,
        _ => {
            state
                .login_throttle
                .record_failure(now_ms, &form.username, ip);
            tracing::info!(?ip, "failed login attempt");
            let body = LoginTemplate {
                csrf_token: &cookie_csrf,
                error: Some("Benutzername oder Passwort ist falsch."),
            }
            .render()?;
            return Ok((StatusCode::UNAUTHORIZED, Html(body)).into_response());
        }
    };

    state.login_throttle.record_success(&form.username, ip);
    let now = auth::now_secs();
    users::purge_expired_sessions(&state.pool, now).await?;
    let token = auth::random_token();
    let csrf = auth::random_token();
    users::create_session(
        &state.pool,
        &auth::token_hash(&token),
        user.id,
        &csrf,
        now,
        now.saturating_add(state.session_ttl_secs),
    )
    .await?;
    tracing::info!(user = %user.username, "login succeeded");

    Ok((
        AppendHeaders([
            (
                header::SET_COOKIE,
                set_cookie(
                    SESSION_COOKIE,
                    &token,
                    state.session_ttl_secs,
                    state.cookie_secure,
                ),
            ),
            (
                header::SET_COOKIE,
                set_cookie(LOGIN_CSRF_COOKIE, "", 0, state.cookie_secure),
            ),
        ]),
        Redirect::to("/"),
    )
        .into_response())
}

pub async fn logout(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Result<Response> {
    user.verify_csrf(&form.csrf_token)?;
    if let Some(token) = cookie_value(&headers, SESSION_COOKIE) {
        users::delete_session(&state.pool, &auth::token_hash(&token)).await?;
    }
    tracing::info!(user = %user.username, "logout");
    Ok((
        AppendHeaders([(
            header::SET_COOKIE,
            set_cookie(SESSION_COOKIE, "", 0, state.cookie_secure),
        )]),
        Redirect::to("/login"),
    )
        .into_response())
}

/// Reads one cookie from the `Cookie` request header(s).
pub(crate) fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|pair| {
            let (k, v) = pair.trim().split_once('=')?;
            (k == name).then(|| v.to_owned())
        })
}

/// Builds a `Set-Cookie` value: always `HttpOnly` and `SameSite=Strict`.
fn set_cookie(name: &str, value: &str, max_age_secs: i64, secure: bool) -> String {
    let mut cookie =
        format!("{name}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age_secs}");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::testutil::*;

    #[tokio::test]
    async fn anonymous_home_redirects_to_login() {
        let h = harness(true).await;
        let reply = send(&h.app, get("/", None)).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        assert_eq!(reply.location(), "/login");
        let reply = send(&h.app, get("/", Some("opennetmon_session=garbage"))).await;
        assert_eq!(reply.location(), "/login");
    }

    #[tokio::test]
    async fn login_form_sets_strict_httponly_csrf_cookie() {
        let h = harness(true).await;
        let reply = send(&h.app, get("/login", None)).await;
        let cookie = reply.set_cookies().remove(0);
        for attr in ["HttpOnly", "SameSite=Strict", "Secure", "Path=/"] {
            assert!(cookie.contains(attr), "{cookie} lacks {attr}");
        }
        assert!(reply.body.contains("Anmelden"));
    }

    #[tokio::test]
    async fn successful_login_sets_hardened_session_cookie() {
        let h = harness(true).await;
        let (cookie, csrf) = open_login(&h).await;
        let form = format!("username=ANNA&password={PASSWORD}&csrf_token={csrf}");
        let reply = send(&h.app, post("/login", Some(&cookie), &form)).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        let session = reply
            .set_cookies()
            .into_iter()
            .find(|c| c.starts_with("opennetmon_session="))
            .unwrap();
        for attr in ["HttpOnly", "SameSite=Strict", "Secure", "Max-Age=604800"] {
            assert!(session.contains(attr), "{session} lacks {attr}");
        }
        // The login CSRF cookie is cleared.
        assert!(
            reply
                .set_cookies()
                .iter()
                .any(|c| c.starts_with("opennetmon_login=;") && c.contains("Max-Age=0"))
        );
        // Only the hash of the token is stored.
        let token = session
            .split(';')
            .next()
            .unwrap()
            .split_once('=')
            .unwrap()
            .1;
        let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE id = ?")
            .bind(token)
            .fetch_one(&h.state.pool)
            .await
            .unwrap();
        assert_eq!(stored, 0);
    }

    #[tokio::test]
    async fn secure_flag_follows_config() {
        let h = harness(false).await;
        let reply = send(&h.app, get("/login", None)).await;
        assert!(!reply.set_cookies()[0].contains("Secure"));
    }

    #[tokio::test]
    async fn wrong_password_and_unknown_user_look_identical() {
        let h = harness(true).await;
        let (cookie, csrf) = open_login(&h).await;
        let wrong = format!("username=anna&password=wrong-password-1&csrf_token={csrf}");
        let unknown = format!("username=nobody&password=wrong-password-1&csrf_token={csrf}");
        let a = send(&h.app, post("/login", Some(&cookie), &wrong)).await;
        let b = send(&h.app, post("/login", Some(&cookie), &unknown)).await;
        assert_eq!(a.status, StatusCode::UNAUTHORIZED);
        assert_eq!(a.status, b.status);
        assert_eq!(a.body, b.body);
        assert!(a.body.contains("Benutzername oder Passwort ist falsch."));
        assert!(a.cookie_pair(SESSION_COOKIE).is_none());
        assert!(b.cookie_pair(SESSION_COOKIE).is_none());
    }

    #[tokio::test]
    async fn login_without_matching_csrf_is_forbidden() {
        let h = harness(true).await;
        let (cookie, csrf) = open_login(&h).await;
        let good = format!("username=anna&password={PASSWORD}&csrf_token=");
        // Missing token in the form.
        let reply = send(&h.app, post("/login", Some(&cookie), &good)).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        // No cookie at all.
        let form = format!("username=anna&password={PASSWORD}&csrf_token={csrf}");
        let reply = send(&h.app, post("/login", None, &form)).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        // Mismatching token.
        let form = format!("username=anna&password={PASSWORD}&csrf_token=other");
        let reply = send(&h.app, post("/login", Some(&cookie), &form)).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn home_shows_user_and_role_when_logged_in() {
        let h = harness(true).await;
        let (session, _) = login(&h, "bert").await;
        let reply = send(&h.app, get("/", Some(&session))).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert!(reply.body.contains("bert"));
        assert!(reply.body.contains("Leser"));
    }

    #[tokio::test]
    async fn expired_session_is_rejected() {
        let h = harness(true).await;
        let (session, _) = login(&h, "anna").await;
        sqlx::query("UPDATE sessions SET expires_at = ?")
            .bind(auth::now_secs() - 1)
            .execute(&h.state.pool)
            .await
            .unwrap();
        let reply = send(&h.app, get("/", Some(&session))).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        assert_eq!(reply.location(), "/login");
    }

    #[tokio::test]
    async fn logout_requires_csrf_and_ends_the_session() {
        let h = harness(true).await;
        let (session, csrf) = login(&h, "anna").await;

        let reply = send(&h.app, post("/logout", Some(&session), "csrf_token=wrong")).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        assert_eq!(
            send(&h.app, get("/", Some(&session))).await.status,
            StatusCode::OK
        );

        let reply = send(
            &h.app,
            post("/logout", Some(&session), &format!("csrf_token={csrf}")),
        )
        .await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        assert_eq!(reply.location(), "/login");
        assert!(reply.set_cookies()[0].contains("Max-Age=0"));
        let after = send(&h.app, get("/", Some(&session))).await;
        assert_eq!(after.location(), "/login");
    }

    #[tokio::test]
    async fn logout_without_session_redirects_to_login() {
        let h = harness(true).await;
        let reply = send(&h.app, post("/logout", None, "csrf_token=x")).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        assert_eq!(reply.location(), "/login");
    }

    #[tokio::test]
    async fn static_assets_are_served_without_login() {
        let h = harness(true).await;
        let reply = send(&h.app, get("/static/app.css", None)).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert!(
            reply.headers[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/css")
        );
        assert_eq!(
            send(&h.app, get("/static/missing.js", None)).await.status,
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn cookie_parsing_handles_multiple_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "a=1; opennetmon_session=tok; b=2".parse().unwrap(),
        );
        assert_eq!(
            cookie_value(&headers, "opennetmon_session").as_deref(),
            Some("tok")
        );
        assert_eq!(cookie_value(&headers, "missing"), None);
    }

    #[test]
    fn viewer_cannot_pass_admin_check() {
        let user = |role| CurrentUser {
            id: 1,
            username: "x".to_owned(),
            role,
            csrf_token: "t".to_owned(),
        };
        assert!(user(Role::Admin).require_admin().is_ok());
        assert!(matches!(
            user(Role::Viewer).require_admin(),
            Err(Error::Forbidden)
        ));
    }

    /// One login attempt; `forwarded` becomes the `X-Forwarded-For` header.
    async fn attempt(
        app: &axum::Router,
        cookie: &str,
        csrf: &str,
        user: &str,
        password: &str,
        forwarded: Option<&str>,
    ) -> Reply {
        let mut request = axum::http::Request::builder()
            .method("POST")
            .uri("/login")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::COOKIE, cookie);
        if let Some(addr) = forwarded {
            request = request.header("x-forwarded-for", addr);
        }
        let body = format!("username={user}&password={password}&csrf_token={csrf}");
        send(app, request.body(axum::body::Body::from(body)).unwrap()).await
    }

    #[tokio::test]
    async fn repeated_failures_for_one_user_are_refused_even_with_the_right_password() {
        let h = harness(true).await;
        let (cookie, csrf) = open_login(&h).await;
        for _ in 0..5 {
            let reply = attempt(&h.app, &cookie, &csrf, "anna", "falsch", None).await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        }
        let blocked = attempt(&h.app, &cookie, &csrf, "anna", PASSWORD, None).await;
        assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS);
        assert!(
            blocked
                .body
                .contains("Zu viele fehlgeschlagene Anmeldeversuche"),
            "{}",
            blocked.body
        );
        assert!(blocked.body.contains("Sekunden"));
        let retry: u32 = blocked.headers[header::RETRY_AFTER]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((1..=5).contains(&retry), "{retry}");
        assert!(
            blocked.cookie_pair(SESSION_COOKIE).is_none(),
            "no session while refused"
        );

        // The refusal is per user name: another user can still log in (and the wrong-password text is unchanged).
        let other = attempt(&h.app, &cookie, &csrf, "bert", PASSWORD, None).await;
        assert_eq!(other.status, StatusCode::SEE_OTHER);
    }

    #[tokio::test]
    async fn a_successful_login_resets_the_counter() {
        let h = harness(true).await;
        let (cookie, csrf) = open_login(&h).await;
        for round in 0..3 {
            for _ in 0..4 {
                let reply = attempt(&h.app, &cookie, &csrf, "anna", "falsch", None).await;
                assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "round {round}");
            }
            let ok = attempt(&h.app, &cookie, &csrf, "anna", PASSWORD, None).await;
            assert_eq!(
                ok.status,
                StatusCode::SEE_OTHER,
                "round {round}: four failures are still free"
            );
        }
    }

    #[tokio::test]
    async fn one_address_cannot_walk_through_many_accounts_but_others_are_unaffected() {
        let mut h = harness(true).await;
        h.state.client_ip_header = Some(throttle::ClientIpHeader::XForwardedFor);
        let app = crate::web::router(h.state.clone());
        let (cookie, csrf) = open_login(&h).await;
        for i in 0..10 {
            let reply = attempt(
                &app,
                &cookie,
                &csrf,
                &format!("nobody{i}"),
                "x",
                Some("198.51.100.1, 203.0.113.7"),
            )
            .await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        }
        let blocked = attempt(
            &app,
            &cookie,
            &csrf,
            "bert",
            PASSWORD,
            Some("6.6.6.6, 203.0.113.7"),
        )
        .await;
        assert_eq!(
            blocked.status,
            StatusCode::TOO_MANY_REQUESTS,
            "the spoofed first entry does not help"
        );
        let elsewhere = attempt(&app, &cookie, &csrf, "bert", PASSWORD, Some("203.0.113.8")).await;
        assert_eq!(elsewhere.status, StatusCode::SEE_OTHER);
    }

    #[tokio::test]
    async fn without_a_configured_header_the_forwarded_header_is_ignored() {
        let h = harness(true).await;
        let (cookie, csrf) = open_login(&h).await;
        for i in 0..10 {
            // Different claimed addresses; all land in the same (unknown-peer) bucket.
            attempt(
                &h.app,
                &cookie,
                &csrf,
                &format!("nobody{i}"),
                "x",
                Some(&format!("203.0.113.{i}")),
            )
            .await;
        }
        let blocked = attempt(
            &h.app,
            &cookie,
            &csrf,
            "bert",
            PASSWORD,
            Some("203.0.113.99"),
        )
        .await;
        assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn waiting_times_are_readable() {
        assert_eq!(wait_text(1), "1 Sekunde");
        assert_eq!(wait_text(5_000), "5 Sekunden");
        assert_eq!(wait_text(59_000), "59 Sekunden");
        assert_eq!(wait_text(60_000), "1 Minute");
        assert_eq!(wait_text(61_000), "2 Minuten");
        assert_eq!(wait_text(900_000), "15 Minuten");
    }
}
