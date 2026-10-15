//! Application error type. Details are logged, clients only get a generic message.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::ConfigError;
use crate::crypto::CryptoError;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("template error: {0}")]
    Template(#[from] askama::Error),
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error("internal error: {0}")]
    Internal(String),
    /// Invalid user input; message is user-facing (German).
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found")]
    NotFound,
    /// Request rejected: missing/invalid CSRF token or insufficient role.
    #[error("forbidden")]
    Forbidden,
    /// User-facing message (German), safe to show.
    #[error("conflict: {0}")]
    Conflict(String),
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        match self {
            Self::Forbidden => (StatusCode::FORBIDDEN, "Zugriff verweigert").into_response(),
            Self::Conflict(message) => (StatusCode::CONFLICT, message).into_response(),
            Self::Invalid(message) => (StatusCode::UNPROCESSABLE_ENTITY, message).into_response(),
            Self::NotFound => (StatusCode::NOT_FOUND, "Nicht gefunden").into_response(),
            other => {
                // Never leak internal details (paths, SQL, secrets) to the client.
                tracing::error!(error = %other, "request failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "Interner Fehler").into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    #[tokio::test]
    async fn response_hides_details() {
        let err = Error::Io(std::io::Error::other("secret path /etc/netmon/key"));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"Interner Fehler");
    }

    #[tokio::test]
    async fn invalid_and_not_found_have_their_own_status() {
        assert_eq!(
            Error::NotFound.into_response().status(),
            StatusCode::NOT_FOUND
        );
        let response = Error::Invalid("zu kurz".to_owned()).into_response();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], "zu kurz".as_bytes());
    }

    #[tokio::test]
    async fn forbidden_and_conflict_have_their_own_status() {
        assert_eq!(
            Error::Forbidden.into_response().status(),
            StatusCode::FORBIDDEN
        );
        let response = Error::Conflict("schon da".to_owned()).into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], "schon da".as_bytes());
    }
}
