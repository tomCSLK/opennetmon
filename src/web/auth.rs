//! Login, logout, session cookies and the `CurrentUser` extractor.

use askama::Template;
use axum::Form;
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{AppendHeaders, Html, IntoResponse, Json, Redirect, Response};
use serde::Deserialize;

use crate::audit::{Action, Kind};
use crate::auth::{self, Role, throttle};
use crate::error::{Error, Result};
use crate::i18n::{self, Lang, Msg};
use crate::storage::users::{self, SessionUser};
use crate::web::audit::{self, Object};
use crate::web::{AppState, Layout};

/// The object of sign-in and sign-out entries in the audit log.
const SESSION: Object<'static> = Object {
    kind: Kind::Session,
    id: None,
    name: "",
};

pub(crate) const SESSION_COOKIE: &str = "opennetmon_session";
pub(crate) const LOGIN_CSRF_COOKIE: &str = "opennetmon_login";
/// Set by `static/refresh.js` for its automatic reloads, which do not count as activity.
pub(crate) const AUTO_RELOAD_COOKIE: &str = "opennetmon_auto";
/// Where a session that was idle for too long ends up.
const SIGNED_OUT_IDLE: &str = "/login?idle=1";
/// Activity is written at most this often per session (seconds).
const TOUCH_EVERY_SECS: i64 = 15;
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
    /// Language of the pages: the user's choice, otherwise the browser's.
    pub lang: Lang,
    /// The stored choice (`None`: follow the browser).
    pub language: Option<Lang>,
    pub csrf_token: String,
    /// Seconds until the sign-out without activity (`None`: no limit for this user).
    pub session_left: Option<i64>,
    /// A newer release (administrators only), see [`crate::update_check`].
    pub update: Option<crate::update_check::Release>,
    /// Address of the client (behind a proxy from `client_ip_header`), for the audit log.
    pub address: Option<std::net::IpAddr>,
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
            role_label: self.lang.get(self.role.label_id()),
            csrf_token: self.csrf_token.clone(),
            is_admin: self.role.is_admin(),
            session_left: self.session_left,
            update: self.update.clone(),
        }
    }
}

impl CurrentUser {
    fn from_session(s: SessionUser, headers: &HeaderMap, session_left: Option<i64>) -> Self {
        Self {
            id: s.user_id,
            username: s.username,
            role: s.role,
            lang: s.language.unwrap_or_else(|| browser_language(headers)),
            language: s.language,
            csrf_token: s.csrf_token,
            session_left,
            update: None,
            address: None,
        }
    }
}

/// Whether a request is something the user did: a form sent (or the activity report of `static/session.js`),
/// or a page opened. Requests of the scripts (charts, tables, the session check) and automatic reloads are not.
fn is_activity(parts: &Parts) -> bool {
    if parts.method != Method::GET && parts.method != Method::HEAD {
        return true;
    }
    let page = match parts.headers.get("sec-fetch-mode") {
        Some(mode) => mode == "navigate",
        None => parts
            .headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/html")),
    };
    page && cookie_value(&parts.headers, AUTO_RELOAD_COOKIE).is_none()
}

/// The preferred supported language of the browser (`Accept-Language`), German otherwise.
pub(crate) fn browser_language(headers: &HeaderMap) -> Lang {
    headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .map_or_else(Lang::default, Lang::from_accept_language)
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
        let hash = auth::token_hash(&token);
        let now = auth::now_secs();
        let session = match users::find_session(&state.pool, &hash, now).await {
            Ok(Some(session)) => session,
            Ok(None) => return Err(Redirect::to("/login").into_response()),
            Err(e) => return Err(e.into_response()),
        };
        let address = throttle::client_ip(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ConnectInfo(addr)| addr.ip()),
            &parts.headers,
            state.client_ip_header,
        );
        let idle = state.session_idle_secs.filter(|_| !session.stay_signed_in);
        let mut last_seen = session.last_seen_at;
        if idle.is_some_and(|limit| now.saturating_sub(last_seen) >= limit) {
            users::delete_session(&state.pool, &hash)
                .await
                .map_err(IntoResponse::into_response)?;
            tracing::info!(user = %session.username, "signed out after a time without activity");
            audit::write(
                state,
                &session.username,
                address.map(|a| a.to_string()),
                Action::SignedOut,
                SESSION,
                &Msg::new("audit-signed-out-idle"),
            )
            .await;
            return Err((
                AppendHeaders([(
                    header::SET_COOKIE,
                    set_cookie(SESSION_COOKIE, "", 0, state.cookie_secure),
                )]),
                Redirect::to(SIGNED_OUT_IDLE),
            )
                .into_response());
        }
        if is_activity(parts) && now.saturating_sub(last_seen) >= TOUCH_EVERY_SECS {
            users::touch_session(&state.pool, &hash, now)
                .await
                .map_err(IntoResponse::into_response)?;
            last_seen = now;
        }
        let left = idle.map(|limit| limit.saturating_sub(now.saturating_sub(last_seen)));
        let mut user = Self::from_session(session, &parts.headers, left);
        user.address = address;
        if user.role.is_admin() {
            user.update = state.latest_release.get();
        }
        // Error pages of this request use the user's language as well.
        i18n::set_current(user.lang);
        Ok(user)
    }
}

/// Answer of `/session` for `static/session.js`: seconds until the sign-out, `null` without a limit.
#[derive(serde::Serialize)]
pub struct SessionState {
    left: Option<i64>,
}

/// How long the session lasts without further activity; does not count as activity itself.
pub async fn session_state(user: CurrentUser) -> Json<SessionState> {
    Json(SessionState {
        left: user.session_left,
    })
}

/// Activity in the browser (click, key press) that did not reach the server otherwise.
pub async fn session_active(
    user: CurrentUser,
    Form(form): Form<CsrfForm>,
) -> Result<Json<SessionState>> {
    user.verify_csrf(&form.csrf_token)?;
    Ok(Json(SessionState {
        left: user.session_left,
    }))
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate<'a> {
    /// Language of the page.
    t: Lang,
    csrf_token: &'a str,
    error: Option<String>,
    /// Why the user is here, e.g. signed out after a time without activity.
    notice: Option<String>,
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

#[derive(Deserialize)]
pub struct LoginQuery {
    idle: Option<String>,
}

pub async fn login_form(
    State(state): State<AppState>,
    Query(query): Query<LoginQuery>,
) -> Result<Response> {
    let csrf = auth::random_token();
    let t = i18n::current();
    let notice = query
        .idle
        .and(state.session_idle_secs)
        .map(|secs| t.get1("login-signed-out-idle", "minutes", secs / 60));
    let body = LoginTemplate {
        t,
        csrf_token: &csrf,
        error: None,
        notice,
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
fn wait_text(t: Lang, ms: i64) -> String {
    let secs = (ms.max(0) + 999) / 1000;
    if secs < 60 {
        t.get1("login-wait-seconds", "seconds", secs)
    } else {
        t.get1("login-wait-minutes", "minutes", (secs + 59) / 60)
    }
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
    // Counts as failed until the password is verified, see `LoginThrottle::begin_attempt`.
    if let Err(wait_ms) = state
        .login_throttle
        .begin_attempt(now_ms, &form.username, ip)
    {
        tracing::warn!(?ip, wait_ms, "login attempt refused (too many failures)");
        let t = i18n::current();
        let body = LoginTemplate {
            t,
            csrf_token: &cookie_csrf,
            error: Some(t.get1("login-throttled", "wait", wait_text(t, wait_ms))),
            notice: None,
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
    let Some(verified) = state.check_password(hash, form.password).await? else {
        tracing::warn!(?ip, "login refused: too many password checks at once");
        let t = i18n::current();
        let body = LoginTemplate {
            t,
            csrf_token: &cookie_csrf,
            error: Some(t.get("login-busy")),
            notice: None,
        }
        .render()?;
        return Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            AppendHeaders([(header::RETRY_AFTER, "5")]),
            Html(body),
        )
            .into_response());
    };

    let user = match user {
        Some(user) if verified => user,
        _ => {
            tracing::info!(?ip, "failed login attempt");
            // Only a known user name goes into the audit log: what was typed might be a password.
            let (name, detail) = match &user {
                Some(user) => (user.username.as_str(), Msg::new("audit-wrong-password")),
                None => ("", Msg::new("audit-unknown-user")),
            };
            let address = ip.map(|a| a.to_string());
            audit::write(
                &state,
                name,
                address,
                Action::SignInFailed,
                SESSION,
                &detail,
            )
            .await;
            let t = i18n::current();
            let body = LoginTemplate {
                t,
                csrf_token: &cookie_csrf,
                error: Some(t.get("login-failed")),
                notice: None,
            }
            .render()?;
            return Ok((StatusCode::UNAUTHORIZED, Html(body)).into_response());
        }
    };

    state.login_throttle.record_success(&form.username, ip);
    let now = auth::now_secs();
    users::purge_expired_sessions(&state.pool, now, state.session_idle_secs).await?;
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
    let address = ip.map(|a| a.to_string());
    let detail = Msg::List(Vec::new());
    audit::write(
        &state,
        &user.username,
        address,
        Action::SignedIn,
        SESSION,
        &detail,
    )
    .await;

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
    let address = user.address.map(|a| a.to_string());
    let detail = Msg::List(Vec::new());
    audit::write(
        &state,
        &user.username,
        address,
        Action::SignedOut,
        SESSION,
        &detail,
    )
    .await;
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
    use std::sync::Arc;

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
    async fn source_code_is_offered_before_and_after_login() {
        let h = harness(true).await;
        let link = "href=\"https://github.com/tomCSLK/opennetmon\"";
        let reply = send(&h.app, get("/login", None)).await;
        assert!(
            reply.body.contains(link),
            "login page lacks the source link"
        );
        let (session, _) = login(&h, "bert").await;
        let reply = send(&h.app, get("/", Some(&session))).await;
        assert!(reply.body.contains(link), "sidebar lacks the source link");
        assert!(reply.body.contains("AGPL-3.0"));
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

    #[tokio::test]
    async fn only_administrators_see_a_newer_version() {
        let h = harness(false).await;
        let (admin, _) = login(&h, "anna").await;
        let (viewer, _) = login(&h, "bert").await;
        let hint = "class=\"sidebar-update\"";
        assert!(
            !send(&h.app, get("/", Some(&admin)))
                .await
                .body
                .contains(hint)
        );

        h.state
            .latest_release
            .set(Some(crate::update_check::Release {
                version: "9.1.0".to_owned(),
                url: "https://github.com/example/opennetmon/releases/tag/v9.1.0".to_owned(),
            }));
        let page = send(&h.app, get("/", Some(&admin))).await.body;
        assert!(page.contains(hint), "{page}");
        assert!(
            page.contains("href=\"https://github.com/example/opennetmon/releases/tag/v9.1.0\"")
        );
        assert!(page.contains("9.1.0"));
        let page = send(&h.app, get("/", Some(&viewer))).await.body;
        assert!(!page.contains(hint));
        assert!(!page.contains("9.1.0"));
    }

    #[test]
    fn viewer_cannot_pass_admin_check() {
        let user = |role| CurrentUser {
            id: 1,
            username: "x".to_owned(),
            role,
            lang: Lang::De,
            language: None,
            csrf_token: "t".to_owned(),
            session_left: None,
            update: None,
            address: None,
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn attempts_sent_at_once_are_refused_like_one_after_the_other() {
        // Each attempt is counted before its password check, so a burst cannot slip past the throttle
        // (once 123 of 200 got through where 10 should).
        let h = harness(true).await;
        let (cookie, csrf) = open_login(&h).await;
        let attempts: Vec<_> = (0..20)
            .map(|_| {
                let (app, cookie, csrf) = (h.app.clone(), cookie.clone(), csrf.clone());
                tokio::spawn(async move {
                    attempt(&app, &cookie, &csrf, "anna", "falsch", None)
                        .await
                        .status
                })
            })
            .collect();
        let mut refused = 0;
        let mut checked = 0;
        for a in attempts {
            match a.await.unwrap() {
                StatusCode::UNAUTHORIZED => checked += 1,
                StatusCode::TOO_MANY_REQUESTS => refused += 1,
                other => panic!("{other}"),
            }
        }
        assert_eq!((checked, refused), (5, 15));
    }

    #[tokio::test]
    async fn sign_ins_wait_for_a_free_password_check() {
        let h = harness(true).await;
        let (cookie, csrf) = open_login(&h).await;
        let all = u32::try_from(crate::web::PASSWORD_CHECKS).unwrap();
        let busy = Arc::clone(&h.state.password_checks)
            .acquire_many_owned(all)
            .await
            .unwrap();
        let app = h.app.clone();
        let (c, t) = (cookie.clone(), csrf.clone());
        let pending =
            tokio::spawn(async move { attempt(&app, &c, &t, "anna", PASSWORD, None).await });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(!pending.is_finished(), "no check without a free slot");
        drop(busy);
        assert_eq!(pending.await.unwrap().status, StatusCode::SEE_OTHER);

        // Without a free slot for a while the attempt is turned away instead of piling up.
        let _busy = Arc::clone(&h.state.password_checks)
            .acquire_many_owned(all)
            .await
            .unwrap();
        tokio::time::pause();
        assert_eq!(
            h.state
                .check_password(auth::dummy_hash().to_owned(), "x".to_owned())
                .await
                .unwrap(),
            None
        );
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
        assert_eq!(wait_text(Lang::De, 1), "1 Sekunde");
        assert_eq!(wait_text(Lang::De, 5_000), "5 Sekunden");
        assert_eq!(wait_text(Lang::De, 59_000), "59 Sekunden");
        assert_eq!(wait_text(Lang::De, 60_000), "1 Minute");
        assert_eq!(wait_text(Lang::De, 61_000), "2 Minuten");
        assert_eq!(wait_text(Lang::De, 900_000), "15 Minuten");
    }

    #[tokio::test]
    async fn the_login_page_and_its_errors_follow_the_browser_language() {
        let h = harness(true).await;
        let page = send(
            &h.app,
            accepting(get("/login", None), "en-GB,en;q=0.8,de;q=0.5"),
        )
        .await;
        assert!(page.body.contains("<html lang=\"en\">"));
        assert!(page.body.contains("Sign in") && page.body.contains("Source code"));
        let german = send(&h.app, get("/login", None)).await;
        assert!(german.body.contains("<html lang=\"de\">") && german.body.contains("Anmelden"));

        let (cookie, csrf) = open_login(&h).await;
        let form = format!("username=anna&password=falsch-falsch&csrf_token={csrf}");
        let failed = send(
            &h.app,
            accepting(post("/login", Some(&cookie), &form), "en"),
        )
        .await;
        assert_eq!(failed.status, StatusCode::UNAUTHORIZED);
        assert!(failed.body.contains("Wrong user name or password."));
        let forged = send(
            &h.app,
            accepting(
                post(
                    "/login",
                    Some(&cookie),
                    "username=anna&password=x&csrf_token=no",
                ),
                "en",
            ),
        )
        .await;
        assert_eq!(forged.status, StatusCode::FORBIDDEN);
        assert_eq!(forged.body, "Access denied");
    }

    #[tokio::test]
    async fn signed_in_users_get_their_own_language_regardless_of_the_browser() {
        let h = harness(true).await;
        users::set_language(&h.state.pool, 1, Some(Lang::En))
            .await
            .unwrap();
        let (admin, _) = login(&h, "anna").await;
        let home = send(&h.app, accepting(get("/", Some(&admin)), "de-DE")).await;
        assert!(home.body.contains("<html lang=\"en\">") && home.body.contains("Sign out"));
        let missing = send(&h.app, accepting(get("/users/999", Some(&admin)), "de")).await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND);
        assert_eq!(missing.body, "Not found");
    }

    #[test]
    fn waiting_times_are_translated() {
        assert_eq!(wait_text(Lang::En, 1), "1 second");
        assert_eq!(wait_text(Lang::En, 120_000), "2 minutes");
    }

    /// Moves the last activity of all sessions `secs` into the past.
    async fn idle_for(h: &Harness, secs: i64) {
        sqlx::query("UPDATE sessions SET last_seen_at = ?")
            .bind(auth::now_secs() - secs)
            .execute(&h.state.pool)
            .await
            .unwrap();
    }

    async fn last_seen(h: &Harness) -> i64 {
        sqlx::query_scalar("SELECT last_seen_at FROM sessions")
            .fetch_one(&h.state.pool)
            .await
            .unwrap()
    }

    /// A request with extra headers, as a browser sends it.
    fn with(
        mut request: axum::http::Request<axum::body::Body>,
        pairs: &[(&'static str, &str)],
    ) -> axum::http::Request<axum::body::Body> {
        for (name, value) in pairs {
            request.headers_mut().append(*name, value.parse().unwrap());
        }
        request
    }

    #[tokio::test]
    async fn idle_sessions_end_with_a_note_on_the_login_page() {
        let h = harness(true).await;
        let (session, _) = login(&h, "bert").await;
        idle_for(&h, 29 * 60).await;
        let page = send(&h.app, get("/", Some(&session))).await;
        assert_eq!(page.status, StatusCode::OK);
        // The page tells session.js how long is left (about a minute).
        let left: i64 = page
            .body
            .split("data-session-left=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap()
            .parse()
            .unwrap();
        assert!((55..=60).contains(&left), "{left}");

        idle_for(&h, 30 * 60).await;
        let reply = send(&h.app, get("/", Some(&session))).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        assert_eq!(reply.location(), "/login?idle=1");
        assert!(reply.set_cookies()[0].starts_with("opennetmon_session=;"));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
            .fetch_one(&h.state.pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
        // Signed out for good: the old cookie does not come back to life.
        let again = send(&h.app, get("/", Some(&session))).await;
        assert_eq!(again.location(), "/login");

        let login_page = send(&h.app, get("/login?idle=1", None)).await;
        assert!(
            login_page
                .body
                .contains("Du wurdest nach 30 Minuten ohne Aktivität abgemeldet."),
            "{}",
            login_page.body
        );
        let plain = send(&h.app, get("/login", None)).await;
        assert!(!plain.body.contains("ohne Aktivität"));
    }

    #[tokio::test]
    async fn only_what_the_user_does_counts_as_activity() {
        let h = harness(true).await;
        let (session, csrf) = login(&h, "bert").await;
        let auto = format!("{session}; {AUTO_RELOAD_COOKIE}=1");
        idle_for(&h, 600).await;
        let before = last_seen(&h).await;

        // Automatic reloads and requests of the scripts do not count.
        let reload = with(
            get("/", Some(&auto)),
            &[("sec-fetch-mode", "navigate"), ("accept", "text/html")],
        );
        assert_eq!(send(&h.app, reload).await.status, StatusCode::OK);
        let script = with(
            get("/session", Some(&session)),
            &[("sec-fetch-mode", "cors"), ("accept", "application/json")],
        );
        let state = send(&h.app, script).await;
        assert_eq!(state.status, StatusCode::OK);
        let left: i64 = state
            .body
            .strip_prefix("{\"left\":")
            .and_then(|rest| rest.strip_suffix('}'))
            .unwrap()
            .parse()
            .unwrap();
        assert!((1190..=1200).contains(&left), "{}", state.body);
        assert_eq!(last_seen(&h).await, before);

        // Opening a page does.
        let page = with(
            get("/", Some(&session)),
            &[("sec-fetch-mode", "navigate"), ("accept", "text/html")],
        );
        send(&h.app, page).await;
        assert!(last_seen(&h).await > before);

        // So does the report of session.js, which needs the CSRF token.
        idle_for(&h, 600).await;
        let forged = send(&h.app, post("/session", Some(&session), "csrf_token=x")).await;
        assert_eq!(forged.status, StatusCode::FORBIDDEN);
        idle_for(&h, 600).await;
        let report = send(
            &h.app,
            post("/session", Some(&session), &format!("csrf_token={csrf}")),
        )
        .await;
        assert_eq!(report.status, StatusCode::OK);
        assert_eq!(report.body, "{\"left\":1800}");
    }

    #[tokio::test]
    async fn users_who_stay_signed_in_and_disabled_limits_have_no_idle_end() {
        let h = harness(true).await;
        let (session, _) = login(&h, "bert").await;
        let bert = users::find_by_username(&h.state.pool, "bert")
            .await
            .unwrap()
            .unwrap();
        users::set_stay_signed_in(&h.state.pool, bert.id, true)
            .await
            .unwrap();
        idle_for(&h, 24 * 3600).await;
        let page = send(&h.app, get("/", Some(&session))).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(!page.body.contains("data-session-left"));
        let state = send(&h.app, get("/session", Some(&session))).await;
        assert_eq!(state.body, "{\"left\":null}");

        let mut state = h.state.clone();
        state.session_idle_secs = None;
        let app = crate::web::router(state);
        let (anna, _) = login(&h, "anna").await;
        idle_for(&h, 24 * 3600).await;
        let page = send(&app, get("/", Some(&anna))).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(!page.body.contains("data-session-left"));
    }
}
