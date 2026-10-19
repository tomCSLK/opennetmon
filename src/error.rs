//! Application error type. Details are logged, clients only get a generic message.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::config::ConfigError;
use crate::crypto::CryptoError;
use crate::i18n::{self, Msg};

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
    /// Invalid user input; the message is user-facing.
    #[error("invalid input: {0}")]
    Invalid(Msg),
    #[error("not found")]
    NotFound,
    /// Request rejected: missing/invalid CSRF token or insufficient role.
    #[error("forbidden")]
    Forbidden,
    /// User-facing message, safe to show.
    #[error("conflict: {0}")]
    Conflict(Msg),
}

impl Error {
    /// Invalid user input with a message (a catalog [`Msg`] or, for now, a German text).
    pub fn invalid(message: impl Into<Msg>) -> Self {
        Self::Invalid(message.into())
    }

    pub fn conflict(message: impl Into<Msg>) -> Self {
        Self::Conflict(message.into())
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let t = i18n::current();
        match self {
            Self::Forbidden => (StatusCode::FORBIDDEN, t.get("error-forbidden")).into_response(),
            Self::Conflict(message) => (StatusCode::CONFLICT, t.msg(&message)).into_response(),
            Self::Invalid(message) => {
                (StatusCode::UNPROCESSABLE_ENTITY, t.msg(&message)).into_response()
            }
            Self::NotFound => (StatusCode::NOT_FOUND, t.get("error-not-found")).into_response(),
            other => {
                // Never leak internal details (paths, SQL, secrets) to the client.
                tracing::error!(error = %other, "request failed");
                (StatusCode::INTERNAL_SERVER_ERROR, t.get("error-internal")).into_response()
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
        let err = Error::Io(std::io::Error::other("secret path /etc/opennetmon/key"));
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
        let response = Error::invalid("zu kurz").into_response();
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
        let response = Error::conflict("schon da").into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], "schon da".as_bytes());
    }
}
