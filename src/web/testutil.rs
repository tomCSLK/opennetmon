//! Shared helpers for handler tests: a ready-made app with two users and request builders.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::auth::{self, Role};
use crate::config::Config;
use crate::crypto::SecretBox;
use crate::storage::{self, users};
use crate::web::auth::{LOGIN_CSRF_COOKIE, SESSION_COOKIE};
use crate::web::{AppState, router};

pub const PASSWORD: &str = "correct-horse-battery";

pub struct Harness {
    pub app: Router,
    pub state: AppState,
    _dir: tempfile::TempDir,
}

pub async fn harness(cookie_secure: bool) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let pool = storage::connect(&dir.path().join("a.db")).await.unwrap();
    storage::migrate(&pool).await.unwrap();
    let hash = auth::hash_password(PASSWORD).unwrap();
    users::create_user(&pool, "anna", &hash, Role::Admin)
        .await
        .unwrap();
    users::create_user(&pool, "bert", &hash, Role::Viewer)
        .await
        .unwrap();
    let config = Config {
        cookie_secure,
        ..Config::default()
    };
    let state = AppState::new(pool, &config, Arc::new(SecretBox::from_key([5u8; 32])));
    Harness {
        app: router(state.clone()),
        state,
        _dir: dir,
    }
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: String,
}

impl Reply {
    pub fn set_cookies(&self) -> Vec<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_owned())
            .collect()
    }

    /// `name=value` pair of the cookie set by this reply.
    pub fn cookie_pair(&self, name: &str) -> Option<String> {
        self.set_cookies()
            .into_iter()
            .find(|c| c.starts_with(&format!("{name}=")))
            .map(|c| c.split(';').next().unwrap().to_owned())
    }

    pub fn location(&self) -> &str {
        self.headers[header::LOCATION].to_str().unwrap()
    }
}

pub async fn send(app: &Router, request: Request<Body>) -> Reply {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        headers,
        body: String::from_utf8(body.to_vec()).unwrap(),
    }
}

pub fn get(path: &str, cookie: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().uri(path);
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    b.body(Body::empty()).unwrap()
}

pub fn post(path: &str, cookie: Option<&str>, form: &str) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    b.body(Body::from(form.to_owned())).unwrap()
}

/// Loads the login form and returns (cookie pair, csrf token).
pub async fn open_login(h: &Harness) -> (String, String) {
    let reply = send(&h.app, get("/login", None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let pair = reply.cookie_pair(LOGIN_CSRF_COOKIE).unwrap();
    let token = pair.split_once('=').unwrap().1.to_owned();
    assert!(reply.body.contains(&format!("value=\"{token}\"")));
    (pair, token)
}

/// Logs in and returns (session cookie pair, session csrf token read from the home page).
pub async fn login(h: &Harness, username: &str) -> (String, String) {
    let (cookie, csrf) = open_login(h).await;
    let form = format!("username={username}&password={PASSWORD}&csrf_token={csrf}");
    let reply = send(&h.app, post("/login", Some(&cookie), &form)).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(reply.location(), "/");
    let session = reply.cookie_pair(SESSION_COOKIE).unwrap();
    let home = send(&h.app, get("/", Some(&session))).await;
    let csrf = home
        .body
        .split("name=\"csrf_token\" value=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_owned();
    (session, csrf)
}

/// Creates an SNMPv2c profile and returns its id.
pub async fn add_v2c_profile(h: &Harness, name: &str, community: &str) -> i64 {
    use crate::credentials::{ProfileInput, ProfileKind, Secret};
    let input = ProfileInput {
        name: name.to_owned(),
        kind: ProfileKind::V2c {
            community: Some(Secret::new(community)),
        },
    };
    storage::credentials::save_profile(&h.state.pool, &h.state.secrets, None, &input)
        .await
        .unwrap()
}
