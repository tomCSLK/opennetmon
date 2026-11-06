//! The read-only API for other programs. Today one endpoint: the inventory of devices that opensecmon
//! reads to suggest its assets (`GET /api/v1/inventory`, see `docs/api.md`).
//!
//! Access is by token only (`Authorization: Bearer <token>`, tokens are made by administrators on
//! `/api-tokens`, see [`super::api_tokens`]): no session, no cookie, no CSRF, nothing that changes
//! anything. Attempts are counted per client address like sign-ins (`AppState::api_throttle`): a bad
//! or missing token counts as a failure, a good one clears the count.

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::header::{AUTHORIZATION, RETRY_AFTER, WWW_AUTHENTICATE};
use axum::http::{Extensions, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::auth::{self, throttle};
use crate::storage::{api_tokens, devices};
use crate::web::AppState;

/// Every token starts with this, so a leaked one can be recognised (and found by secret scanners).
pub const TOKEN_PREFIX: &str = "onm_";
/// No real token is longer; anything longer is refused without hashing it.
const MAX_TOKEN_CHARS: usize = 128;

/// A new token: the prefix and 256 random bits as hex.
pub fn new_token() -> String {
    format!("{TOKEN_PREFIX}{}", auth::random_token())
}

#[derive(Serialize)]
struct Inventory {
    devices: Vec<InventoryDevice>,
}

#[derive(Serialize)]
struct InventoryDevice {
    id: i64,
    name: String,
    address: String,
    sys_descr: String,
    sys_object_id: String,
}

fn error(status: StatusCode, message: &'static str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

fn unauthorized() -> Response {
    let mut reply = error(StatusCode::UNAUTHORIZED, "unauthorized");
    reply
        .headers_mut()
        .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    reply
}

/// The token of an `Authorization: Bearer` header, if there is a plausible one.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer")
        && token.starts_with(TOKEN_PREFIX)
        && token.chars().count() <= MAX_TOKEN_CHARS)
        .then_some(token)
}

/// `GET /api/v1/inventory`: all devices (paused ones too) with their name, address and what they
/// said about themselves; `sys_descr` and `sys_object_id` are empty for devices that never answered
/// (or have no SNMP).
pub async fn inventory(
    State(state): State<AppState>,
    headers: HeaderMap,
    extensions: Extensions,
) -> Response {
    let ip = throttle::client_ip(
        extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip()),
        &headers,
        state.client_ip_header,
    );
    let key = ip.map(|ip| ip.to_string()).unwrap_or_default();
    let now_ms = auth::now_secs().saturating_mul(1000);
    // Counts as failed until the token is found, see `LoginThrottle::begin_attempt`.
    if let Err(wait_ms) = state.api_throttle.begin_attempt(now_ms, &key, ip) {
        tracing::warn!(?ip, wait_ms, "API request refused (too many failures)");
        let mut reply = error(StatusCode::TOO_MANY_REQUESTS, "too many failed attempts");
        if let Ok(secs) = HeaderValue::from_str(&((wait_ms + 999) / 1000).to_string()) {
            reply.headers_mut().insert(RETRY_AFTER, secs);
        }
        return reply;
    }
    let Some(token) = bearer(&headers) else {
        return unauthorized();
    };
    match api_tokens::use_token(&state.pool, &auth::token_hash(token), now_ms).await {
        Ok(Some(_)) => state.api_throttle.record_success(&key, ip),
        Ok(None) => {
            tracing::warn!(?ip, "API request with an unknown token");
            return unauthorized();
        }
        Err(e) => return e.into_response(),
    }
    match devices::list_inventory(&state.pool).await {
        Ok(list) => Json(Inventory {
            devices: list
                .into_iter()
                .map(|d| InventoryDevice {
                    id: d.id,
                    name: d.name,
                    address: d.address,
                    sys_descr: d.sys_descr,
                    sys_object_id: d.sys_object_id,
                })
                .collect(),
        })
        .into_response(),
        Err(e) => e.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, header};

    use super::*;
    use crate::devices::{DeviceInput, Polling};
    use crate::snmp::Identity;
    use crate::storage::devices::{save_device, set_identity};
    use crate::web::testutil::*;

    /// A GET with `Authorization: Bearer <token>`.
    fn with_token(path: &str, token: &str) -> Request<Body> {
        Request::builder()
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    async fn add_token(h: &Harness, name: &str) -> String {
        let token = new_token();
        api_tokens::create(
            &h.state.pool,
            name,
            &auth::token_hash(&token),
            "anna",
            1_000,
        )
        .await
        .unwrap();
        token
    }

    async fn add_device(h: &Harness, name: &str, host: &str) -> i64 {
        save_device(
            &h.state.pool,
            None,
            DeviceInput {
                kind: Default::default(),
                name: name.to_owned(),
                host: host.to_owned(),
                group_id: None,
                profile_id: None,
                polling: Polling::default(),
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn the_inventory_lists_all_devices_with_what_they_said_about_themselves() {
        let h = harness(true).await;
        let router = add_device(&h, "Router", "192.0.2.1").await;
        add_device(&h, "Printer", "printer.lan").await;
        set_identity(
            &h.state.pool,
            router,
            &Identity {
                sys_descr: "Omada ER605".to_owned(),
                sys_object_id: "1.3.6.1.4.1.11863.5".to_owned(),
            },
            5_000,
        )
        .await
        .unwrap();
        let token = add_token(&h, "opensecmon").await;

        let reply = send(&h.app, with_token("/api/v1/inventory", &token)).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert!(
            reply.headers[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("application/json")
        );
        let json: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "devices": [
                { "id": router, "name": "Router", "address": "192.0.2.1",
                  "sys_descr": "Omada ER605", "sys_object_id": "1.3.6.1.4.1.11863.5" },
                { "id": router + 1, "name": "Printer", "address": "printer.lan",
                  "sys_descr": "", "sys_object_id": "" },
            ]})
        );
        // The token is noted as used.
        let used: Option<i64> = sqlx::query_scalar("SELECT last_used_ms FROM api_tokens")
            .fetch_one(&h.state.pool)
            .await
            .unwrap();
        assert!(used.is_some_and(|ms| ms > 1_000));
    }

    #[tokio::test]
    async fn without_a_valid_token_there_is_no_inventory() {
        let h = harness(true).await;
        add_device(&h, "Router", "192.0.2.1").await;
        let token = add_token(&h, "opensecmon").await;

        // Four failures stay below the point where attempts are refused.
        for request in [
            get("/api/v1/inventory", None),
            with_token("/api/v1/inventory", "onm_wrong"),
            with_token("/api/v1/inventory", &format!("{token}0")),
            // A token of the right kind in a wrong scheme.
            Request::builder()
                .uri("/api/v1/inventory")
                .header(header::AUTHORIZATION, format!("Basic {token}"))
                .body(Body::empty())
                .unwrap(),
        ] {
            let reply = send(&h.app, request).await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.body);
            assert_eq!(reply.headers[header::WWW_AUTHENTICATE], "Bearer");
            assert!(!reply.body.contains("Router"));
        }

        // Another instance, so the failures above do not count: an overlong token and a browser session.
        let h = harness(true).await;
        let long = send(&h.app, with_token("/api/v1/inventory", &"onm_".repeat(100))).await;
        assert_eq!(long.status, StatusCode::UNAUTHORIZED);
        let (admin, _) = login(&h, "anna").await;
        let reply = send(&h.app, get("/api/v1/inventory", Some(&admin))).await;
        assert_eq!(
            reply.status,
            StatusCode::UNAUTHORIZED,
            "a session is no token"
        );
    }

    #[tokio::test]
    async fn wrong_tokens_are_slowed_down_and_a_right_one_is_refused_meanwhile() {
        let h = harness(true).await;
        let token = add_token(&h, "opensecmon").await;
        // Five failures from one address (here: none known), then it is refused for a while.
        for _ in 0..5 {
            let reply = send(&h.app, with_token("/api/v1/inventory", "onm_wrong")).await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        }
        let refused = send(&h.app, with_token("/api/v1/inventory", &token)).await;
        assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
        let wait: u32 = refused.headers[header::RETRY_AFTER]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((1..=60).contains(&wait), "{wait}");
        assert!(!refused.body.contains("devices"));
    }

    #[tokio::test]
    async fn a_good_token_clears_earlier_failures() {
        let h = harness(true).await;
        let token = add_token(&h, "opensecmon").await;
        for _ in 0..3 {
            send(&h.app, with_token("/api/v1/inventory", "onm_wrong")).await;
        }
        assert_eq!(
            send(&h.app, with_token("/api/v1/inventory", &token))
                .await
                .status,
            StatusCode::OK
        );
        // Three more failures would have made six without the reset, which refuses.
        for _ in 0..3 {
            send(&h.app, with_token("/api/v1/inventory", "onm_wrong")).await;
        }
        assert_eq!(
            send(&h.app, with_token("/api/v1/inventory", &token))
                .await
                .status,
            StatusCode::OK
        );
    }

    #[test]
    fn tokens_are_recognisable_and_long() {
        let a = new_token();
        assert!(a.starts_with("onm_") && a.len() == 4 + 64, "{a}");
        assert_ne!(a, new_token());
    }
}
