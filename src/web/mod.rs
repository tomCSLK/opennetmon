//! Web layer: axum router, shared state and handlers.

mod assets;
pub mod auth;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::routing::{get, post};
use sqlx::SqlitePool;
use tower_http::compression::CompressionLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::storage;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub cookie_secure: bool,
    pub session_ttl_secs: i64,
}

impl AppState {
    pub fn new(pool: SqlitePool, config: &Config) -> Self {
        Self {
            pool,
            cookie_secure: config.cookie_secure,
            session_ttl_secs: i64::try_from(config.session_ttl_hours.saturating_mul(3600))
                .unwrap_or(i64::MAX),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(auth::home))
        .route("/login", get(auth::login_form).post(auth::login_submit))
        .route("/logout", post(auth::logout))
        .route("/healthz", get(healthz))
        .route("/static/{*path}", get(assets::serve))
        .layer(security_header(header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(security_header(header::X_FRAME_OPTIONS, "DENY"))
        .layer(security_header(header::REFERRER_POLICY, "same-origin"))
        .layer(security_header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'self'; frame-ancestors 'none'; form-action 'self'",
        ))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

fn security_header(name: HeaderName, value: &'static str) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value))
}

/// Unauthenticated health check; exposes no internal details.
async fn healthz(State(state): State<AppState>) -> (StatusCode, &'static str) {
    match storage::ping(&state.pool).await {
        Ok(()) => (StatusCode::OK, "ok"),
        Err(e) => {
            tracing::warn!(error = %e, "health check failed");
            (StatusCode::SERVICE_UNAVAILABLE, "unavailable")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn test_state() -> (AppState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pool = storage::connect(&dir.path().join("t.db")).await.unwrap();
        storage::migrate(&pool).await.unwrap();
        (AppState::new(pool, &Config::default()), dir)
    }

    async fn get_path(app: Router, path: &str) -> (StatusCode, String) {
        let response = app
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn healthz_is_ok_without_auth() {
        let (state, _dir) = test_state().await;
        let (status, body) = get_path(router(state), "/healthz").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "ok");
    }

    #[tokio::test]
    async fn healthz_reports_unavailable_when_db_is_closed() {
        let (state, _dir) = test_state().await;
        state.pool.close().await;
        let (status, body) = get_path(router(state), "/healthz").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body, "unavailable");
    }

    #[tokio::test]
    async fn unknown_path_is_404() {
        let (state, _dir) = test_state().await;
        let (status, _) = get_path(router(state), "/nope").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn responses_carry_security_headers() {
        let (state, _dir) = test_state().await;
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let h = response.headers();
        assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(h[header::X_FRAME_OPTIONS], "DENY");
        assert!(
            h[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .contains("frame-ancestors 'none'")
        );
    }
}
