//! User and session persistence.

use sqlx::{FromRow, SqlitePool};

use crate::auth::Role;
use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub role: Role,
}

#[derive(FromRow)]
struct UserRow {
    id: i64,
    username: String,
    password_hash: String,
    role: String,
}

impl TryFrom<UserRow> for User {
    type Error = Error;

    fn try_from(row: UserRow) -> Result<Self> {
        let role = Role::parse(&row.role)
            .ok_or_else(|| Error::Internal(format!("unknown role in database: {}", row.role)))?;
        Ok(Self {
            id: row.id,
            username: row.username,
            password_hash: row.password_hash,
            role,
        })
    }
}

/// Inserts a user; usernames are unique case-insensitively.
pub async fn create_user(
    pool: &SqlitePool,
    username: &str,
    password_hash: &str,
    role: Role,
) -> Result<i64> {
    let result = sqlx::query("INSERT INTO users (username, password_hash, role) VALUES (?, ?, ?)")
        .bind(username)
        .bind(password_hash)
        .bind(role.as_str())
        .execute(pool)
        .await;
    match result {
        Ok(done) => Ok(done.last_insert_rowid()),
        Err(e)
            if e.as_database_error()
                .is_some_and(|d| d.is_unique_violation()) =>
        {
            Err(Error::Conflict(
                "Der Benutzername ist bereits vergeben.".to_owned(),
            ))
        }
        Err(e) => Err(e.into()),
    }
}

pub async fn find_by_username(pool: &SqlitePool, username: &str) -> Result<Option<User>> {
    sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash, role FROM users WHERE username = ?",
    )
    .bind(username)
    .fetch_optional(pool)
    .await?
    .map(User::try_from)
    .transpose()
}

pub async fn list_users(pool: &SqlitePool) -> Result<Vec<User>> {
    sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash, role FROM users ORDER BY username",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(User::try_from)
    .collect()
}

pub async fn count_users(pool: &SqlitePool) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?)
}

/// Sets a new password hash; returns false if the user does not exist.
pub async fn set_password_hash(pool: &SqlitePool, username: &str, hash: &str) -> Result<bool> {
    let done = sqlx::query("UPDATE users SET password_hash = ? WHERE username = ?")
        .bind(hash)
        .bind(username)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// The user and CSRF token behind a valid session.
#[derive(Debug, Clone)]
pub struct SessionUser {
    pub user_id: i64,
    pub username: String,
    pub role: Role,
    pub csrf_token: String,
}

#[derive(FromRow)]
struct SessionRow {
    user_id: i64,
    username: String,
    role: String,
    csrf_token: String,
}

pub async fn create_session(
    pool: &SqlitePool,
    token_hash: &str,
    user_id: i64,
    csrf_token: &str,
    now: i64,
    expires_at: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO sessions (id, user_id, csrf_token, created_at, expires_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(token_hash)
    .bind(user_id)
    .bind(csrf_token)
    .bind(now)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Looks up an unexpired session by the hash of its cookie token.
pub async fn find_session(
    pool: &SqlitePool,
    token_hash: &str,
    now: i64,
) -> Result<Option<SessionUser>> {
    let row = sqlx::query_as::<_, SessionRow>(
        "SELECT u.id AS user_id, u.username, u.role, s.csrf_token \
         FROM sessions s JOIN users u ON u.id = s.user_id \
         WHERE s.id = ? AND s.expires_at > ?",
    )
    .bind(token_hash)
    .bind(now)
    .fetch_optional(pool)
    .await?;
    row.map(|r| {
        let role = Role::parse(&r.role)
            .ok_or_else(|| Error::Internal(format!("unknown role in database: {}", r.role)))?;
        Ok(SessionUser {
            user_id: r.user_id,
            username: r.username,
            role,
            csrf_token: r.csrf_token,
        })
    })
    .transpose()
}

pub async fn delete_session(pool: &SqlitePool, token_hash: &str) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE id = ?")
        .bind(token_hash)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_sessions_of_user(pool: &SqlitePool, username: &str) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE user_id IN (SELECT id FROM users WHERE username = ?)")
        .bind(username)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn purge_expired_sessions(pool: &SqlitePool, now: i64) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

    async fn pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pool = storage::connect(&dir.path().join("u.db")).await.unwrap();
        storage::migrate(&pool).await.unwrap();
        (pool, dir)
    }

    #[tokio::test]
    async fn usernames_are_unique_case_insensitively() {
        let (pool, _dir) = pool().await;
        create_user(&pool, "Admin", "hash", Role::Admin)
            .await
            .unwrap();
        let err = create_user(&pool, "admin", "hash", Role::Viewer)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Conflict(_)));
        assert_eq!(count_users(&pool).await.unwrap(), 1);
        let found = find_by_username(&pool, "ADMIN").await.unwrap().unwrap();
        assert_eq!(found.username, "Admin");
        assert_eq!(found.role, Role::Admin);
    }

    #[tokio::test]
    async fn sessions_expire_and_cascade() {
        let (pool, _dir) = pool().await;
        let id = create_user(&pool, "anna", "hash", Role::Viewer)
            .await
            .unwrap();
        create_session(&pool, "h1", id, "csrf", 100, 200)
            .await
            .unwrap();

        let s = find_session(&pool, "h1", 150).await.unwrap().unwrap();
        assert_eq!((s.username.as_str(), s.role), ("anna", Role::Viewer));
        assert!(find_session(&pool, "h1", 200).await.unwrap().is_none());
        assert!(find_session(&pool, "other", 150).await.unwrap().is_none());

        purge_expired_sessions(&pool, 200).await.unwrap();
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 0);

        create_session(&pool, "h2", id, "csrf", 100, 900)
            .await
            .unwrap();
        delete_sessions_of_user(&pool, "anna").await.unwrap();
        assert!(find_session(&pool, "h2", 150).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn set_password_hash_reports_missing_user() {
        let (pool, _dir) = pool().await;
        create_user(&pool, "anna", "old", Role::Viewer)
            .await
            .unwrap();
        assert!(set_password_hash(&pool, "anna", "new").await.unwrap());
        assert!(!set_password_hash(&pool, "nobody", "new").await.unwrap());
        let u = find_by_username(&pool, "anna").await.unwrap().unwrap();
        assert_eq!(u.password_hash, "new");
    }
}
