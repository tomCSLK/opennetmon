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

/// A user for the user list: role, creation time and open sessions.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct UserInfo {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub created_at: String,
    pub sessions: i64,
}

/// All users by name with their number of unexpired sessions.
pub async fn list_user_infos(pool: &SqlitePool, now: i64) -> Result<Vec<UserInfo>> {
    let mut list = sqlx::query_as::<_, UserInfo>(
        "SELECT u.id, u.username, u.role, u.created_at, \
         (SELECT COUNT(*) FROM sessions s WHERE s.user_id = u.id AND s.expires_at > ?) AS sessions \
         FROM users u",
    )
    .bind(now)
    .fetch_all(pool)
    .await?;
    list.sort_by(|a, b| crate::ordering::natural_cmp(&a.username, &b.username));
    Ok(list)
}

pub async fn get_user(pool: &SqlitePool, id: i64) -> Result<Option<User>> {
    sqlx::query_as::<_, UserRow>("SELECT id, username, password_hash, role FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .map(User::try_from)
        .transpose()
}

fn last_admin() -> Error {
    Error::Conflict(
        "Das ist der letzte Administrator. Lege zuerst einen weiteren Administrator an.".to_owned(),
    )
}

/// Changes the role. The last administrator cannot become a viewer (checked in the same statement,
/// so two simultaneous changes cannot remove both admins).
pub async fn set_role(pool: &SqlitePool, id: i64, role: Role) -> Result<()> {
    let done = sqlx::query(
        "UPDATE users SET role = ?1 WHERE id = ?2 AND NOT (role = 'admin' AND ?1 <> 'admin' \
         AND (SELECT COUNT(*) FROM users WHERE role = 'admin') <= 1)",
    )
    .bind(role.as_str())
    .bind(id)
    .execute(pool)
    .await?;
    if done.rows_affected() == 0 {
        return Err(match get_user(pool, id).await? {
            Some(_) => last_admin(),
            None => Error::NotFound,
        });
    }
    Ok(())
}

/// Deletes a user and (by cascade) their sessions; never the last administrator.
pub async fn delete_user(pool: &SqlitePool, id: i64) -> Result<()> {
    let done = sqlx::query(
        "DELETE FROM users WHERE id = ? AND NOT (role = 'admin' \
         AND (SELECT COUNT(*) FROM users WHERE role = 'admin') <= 1)",
    )
    .bind(id)
    .execute(pool)
    .await?;
    if done.rows_affected() == 0 {
        return Err(match get_user(pool, id).await? {
            Some(_) => last_admin(),
            None => Error::NotFound,
        });
    }
    Ok(())
}

/// Sets a new password hash and ends the user's sessions except `keep` (the hash of the session
/// that made the change, so admins changing their own password stay signed in).
pub async fn change_password(
    pool: &SqlitePool,
    id: i64,
    hash: &str,
    keep: Option<&str>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let done = sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(hash)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if done.rows_affected() == 0 {
        return Err(Error::NotFound);
    }
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND id IS NOT ?")
        .bind(id)
        .bind(keep)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
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

    #[tokio::test]
    async fn the_last_admin_can_be_neither_demoted_nor_deleted() {
        let (pool, _dir) = pool().await;
        let a = create_user(&pool, "anna", "h", Role::Admin).await.unwrap();
        let b = create_user(&pool, "bert", "h", Role::Viewer).await.unwrap();
        assert!(matches!(
            set_role(&pool, a, Role::Viewer).await,
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            delete_user(&pool, a).await,
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            set_role(&pool, 99, Role::Admin).await,
            Err(Error::NotFound)
        ));
        set_role(&pool, b, Role::Admin).await.unwrap();
        set_role(&pool, a, Role::Viewer).await.unwrap();
        assert!(matches!(
            delete_user(&pool, b).await,
            Err(Error::Conflict(_))
        ));
        delete_user(&pool, a).await.unwrap();
        assert!(matches!(delete_user(&pool, a).await, Err(Error::NotFound)));
        let infos = list_user_infos(&pool, 0).await.unwrap();
        assert_eq!(
            (infos.len(), infos[0].role.as_str(), infos[0].sessions),
            (1, "admin", 0)
        );
    }

    #[tokio::test]
    async fn changing_a_password_ends_all_sessions_but_the_kept_one() {
        let (pool, _dir) = pool().await;
        let a = create_user(&pool, "anna", "old", Role::Admin)
            .await
            .unwrap();
        for token in ["s1", "s2", "s3"] {
            create_session(&pool, token, a, "c", 0, 100).await.unwrap();
        }
        change_password(&pool, a, "new", Some("s2")).await.unwrap();
        assert!(find_session(&pool, "s1", 1).await.unwrap().is_none());
        assert!(find_session(&pool, "s2", 1).await.unwrap().is_some());
        assert_eq!(
            get_user(&pool, a).await.unwrap().unwrap().password_hash,
            "new"
        );
        change_password(&pool, a, "newer", None).await.unwrap();
        assert!(find_session(&pool, "s2", 1).await.unwrap().is_none());
        assert!(matches!(
            change_password(&pool, 99, "x", None).await,
            Err(Error::NotFound)
        ));
    }
}
