//! Persistence of the access tokens of the read-only API: table `api_tokens`. Only the SHA-256 of a
//! token is stored (like sessions), so a copy of the database does not give access.

use sqlx::SqlitePool;

use crate::error::{Error, Result};
use crate::i18n::Msg;

/// Longest token name, in characters (the table checks the same).
pub const NAME_MAX_CHARS: usize = 64;

/// A token as the admin page lists it; never the token itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiToken {
    pub id: i64,
    pub name: String,
    pub created_by: String,
    pub created_ms: i64,
    pub last_used_ms: Option<i64>,
}

/// Checks the name of a new token: 1 to 64 characters after trimming, no control characters.
pub fn validate_name(name: &str) -> std::result::Result<String, Msg> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > NAME_MAX_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(Msg::new("api-token-name-invalid"));
    }
    Ok(name.to_owned())
}

/// Stores a new token under the hash of its text and returns its id. A name already in use is a
/// conflict.
pub async fn create(
    pool: &SqlitePool,
    name: &str,
    token_hash: &str,
    created_by: &str,
    now_ms: i64,
) -> Result<i64> {
    sqlx::query_scalar(
        "INSERT INTO api_tokens (name, token_hash, created_by, created_ms) VALUES (?, ?, ?, ?) RETURNING id",
    )
    .bind(name)
    .bind(token_hash)
    .bind(created_by)
    .bind(now_ms)
    .fetch_one(pool)
    .await
    .map_err(|e| match e.as_database_error() {
        Some(d) if d.is_unique_violation() => Error::conflict(Msg::new("api-token-name-taken")),
        _ => e.into(),
    })
}

/// All tokens, oldest first.
pub async fn list(pool: &SqlitePool) -> Result<Vec<ApiToken>> {
    let rows: Vec<(i64, String, String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, name, created_by, created_ms, last_used_ms FROM api_tokens ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, name, created_by, created_ms, last_used_ms)| ApiToken {
                id,
                name,
                created_by,
                created_ms,
                last_used_ms,
            },
        )
        .collect())
}

pub async fn get(pool: &SqlitePool, id: i64) -> Result<Option<ApiToken>> {
    Ok(list(pool).await?.into_iter().find(|t| t.id == id))
}

/// Revokes a token; `false` if there was none with this id.
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<bool> {
    let done = sqlx::query("DELETE FROM api_tokens WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// The token with this hash, if any; notes the time of use.
pub async fn use_token(pool: &SqlitePool, token_hash: &str, now_ms: i64) -> Result<Option<i64>> {
    Ok(sqlx::query_scalar(
        "UPDATE api_tokens SET last_used_ms = ? WHERE token_hash = ? RETURNING id",
    )
    .bind(now_ms)
    .bind(token_hash)
    .fetch_optional(pool)
    .await?)
}
