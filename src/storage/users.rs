//! User and session persistence.

use sqlx::{FromRow, SqlitePool};

use crate::auth::Role;
use crate::error::{Error, Result};
use crate::i18n::{Lang, Msg};

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub role: Role,
    /// Chosen language; `None` follows the browser.
    pub language: Option<Lang>,
    /// Exempt from the sign-out after a time without activity (e.g. a wall display).
    pub stay_signed_in: bool,
}

#[derive(FromRow)]
struct UserRow {
    id: i64,
    username: String,
    password_hash: String,
    role: String,
    language: Option<String>,
    stay_signed_in: bool,
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
            language: row.language.as_deref().and_then(Lang::parse),
            stay_signed_in: row.stay_signed_in,
        })
    }
}

/// Columns of [`UserRow`].
const USER_COLUMNS: &str = "id, username, password_hash, role, language, stay_signed_in";

/// SQL condition for sessions (alias `s`, owner `u`) still in use at `?1` (now): not past their lifetime and,
/// with an idle limit `?2` (seconds, NULL: none), active within it unless the owner stays signed in.
const SESSION_ALIVE: &str =
    "s.expires_at > ?1 AND (?2 IS NULL OR u.stay_signed_in = 1 OR s.last_seen_at > ?1 - ?2)";

/// Inserts a user; usernames are unique case-insensitively.
pub async fn create_user(
    pool: &SqlitePool,
    username: &str,
    password_hash: &str,
    role: Role,
    language: Option<Lang>,
) -> Result<i64> {
    let result = sqlx::query(
        "INSERT INTO users (username, password_hash, role, language) VALUES (?, ?, ?, ?)",
    )
    .bind(username)
    .bind(password_hash)
    .bind(role.as_str())
    .bind(language.map(Lang::code))
    .execute(pool)
    .await;
    match result {
        Ok(done) => Ok(done.last_insert_rowid()),
        Err(e)
            if e.as_database_error()
                .is_some_and(|d| d.is_unique_violation()) =>
        {
            Err(Error::conflict(Msg::new("username-taken")))
        }
        Err(e) => Err(e.into()),
    }
}

pub async fn find_by_username(pool: &SqlitePool, username: &str) -> Result<Option<User>> {
    sqlx::query_as::<_, UserRow>(&format!(
        "SELECT {USER_COLUMNS} FROM users WHERE username = ?"
    ))
    .bind(username)
    .fetch_optional(pool)
    .await?
    .map(User::try_from)
    .transpose()
}

pub async fn list_users(pool: &SqlitePool) -> Result<Vec<User>> {
    sqlx::query_as::<_, UserRow>(&format!(
        "SELECT {USER_COLUMNS} FROM users ORDER BY username"
    ))
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

/// A user for the user list: role, language, creation time and open sessions.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct UserInfo {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub language: Option<String>,
    pub stay_signed_in: bool,
    pub created_at: String,
    pub sessions: i64,
}

/// All users by name with their number of sessions still in use (`idle_secs`: the idle limit, if any).
pub async fn list_user_infos(
    pool: &SqlitePool,
    now: i64,
    idle_secs: Option<i64>,
) -> Result<Vec<UserInfo>> {
    let mut list = sqlx::query_as::<_, UserInfo>(&format!(
        "SELECT u.id, u.username, u.role, u.language, u.stay_signed_in, u.created_at, \
         (SELECT COUNT(*) FROM sessions s WHERE s.user_id = u.id AND {SESSION_ALIVE}) AS sessions \
         FROM users u"
    ))
    .bind(now)
    .bind(idle_secs)
    .fetch_all(pool)
    .await?;
    list.sort_by(|a, b| crate::ordering::natural_cmp(&a.username, &b.username));
    Ok(list)
}

pub async fn get_user(pool: &SqlitePool, id: i64) -> Result<Option<User>> {
    sqlx::query_as::<_, UserRow>(&format!("SELECT {USER_COLUMNS} FROM users WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .map(User::try_from)
        .transpose()
}

fn last_admin() -> Error {
    Error::conflict(Msg::new("user-last-admin"))
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

/// Sets the language of a user (`None`: follow the browser).
pub async fn set_language(pool: &SqlitePool, id: i64, language: Option<Lang>) -> Result<()> {
    let done = sqlx::query("UPDATE users SET language = ? WHERE id = ?")
        .bind(language.map(Lang::code))
        .bind(id)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

/// Sets whether a user is exempt from the sign-out after a time without activity.
pub async fn set_stay_signed_in(pool: &SqlitePool, id: i64, stay: bool) -> Result<()> {
    let done = sqlx::query("UPDATE users SET stay_signed_in = ? WHERE id = ?")
        .bind(stay)
        .bind(id)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

/// The user and CSRF token behind a valid session.
#[derive(Debug, Clone)]
pub struct SessionUser {
    pub user_id: i64,
    pub username: String,
    pub role: Role,
    pub language: Option<Lang>,
    pub stay_signed_in: bool,
    pub csrf_token: String,
    /// Last activity (Unix seconds).
    pub last_seen_at: i64,
}

#[derive(FromRow)]
struct SessionRow {
    user_id: i64,
    username: String,
    role: String,
    language: Option<String>,
    stay_signed_in: bool,
    csrf_token: String,
    last_seen_at: i64,
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
        "INSERT INTO sessions (id, user_id, csrf_token, created_at, expires_at, last_seen_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?4)",
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

/// Looks up a session within its lifetime by the hash of its cookie token; the idle limit is up to the caller.
pub async fn find_session(
    pool: &SqlitePool,
    token_hash: &str,
    now: i64,
) -> Result<Option<SessionUser>> {
    let row = sqlx::query_as::<_, SessionRow>(
        "SELECT u.id AS user_id, u.username, u.role, u.language, u.stay_signed_in, \
         s.csrf_token, s.last_seen_at \
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
            language: r.language.as_deref().and_then(Lang::parse),
            stay_signed_in: r.stay_signed_in,
            csrf_token: r.csrf_token,
            last_seen_at: r.last_seen_at,
        })
    })
    .transpose()
}

/// Records activity of a session.
pub async fn touch_session(pool: &SqlitePool, token_hash: &str, now: i64) -> Result<()> {
    sqlx::query("UPDATE sessions SET last_seen_at = ? WHERE id = ?")
        .bind(now)
        .bind(token_hash)
        .execute(pool)
        .await?;
    Ok(())
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

/// Deletes sessions past their lifetime or, with an idle limit (`idle_secs`), unused for longer.
pub async fn purge_expired_sessions(
    pool: &SqlitePool,
    now: i64,
    idle_secs: Option<i64>,
) -> Result<()> {
    sqlx::query(&format!(
        "DELETE FROM sessions AS s WHERE NOT EXISTS \
         (SELECT 1 FROM users u WHERE u.id = s.user_id AND {SESSION_ALIVE})"
    ))
    .bind(now)
    .bind(idle_secs)
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
        create_user(&pool, "Admin", "hash", Role::Admin, None)
            .await
            .unwrap();
        let err = create_user(&pool, "admin", "hash", Role::Viewer, None)
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
        let id = create_user(&pool, "anna", "hash", Role::Viewer, None)
            .await
            .unwrap();
        create_session(&pool, "h1", id, "csrf", 100, 200)
            .await
            .unwrap();

        let s = find_session(&pool, "h1", 150).await.unwrap().unwrap();
        assert_eq!((s.username.as_str(), s.role), ("anna", Role::Viewer));
        assert!(find_session(&pool, "h1", 200).await.unwrap().is_none());
        assert!(find_session(&pool, "other", 150).await.unwrap().is_none());

        purge_expired_sessions(&pool, 200, None).await.unwrap();
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
    async fn idle_sessions_end_unless_the_user_stays_signed_in() {
        let (pool, _dir) = pool().await;
        let anna = create_user(&pool, "anna", "h", Role::Admin, None)
            .await
            .unwrap();
        let wall = create_user(&pool, "wall", "h", Role::Viewer, None)
            .await
            .unwrap();
        set_stay_signed_in(&pool, wall, true).await.unwrap();
        assert!(get_user(&pool, wall).await.unwrap().unwrap().stay_signed_in);
        assert!(matches!(
            set_stay_signed_in(&pool, 99, true).await,
            Err(Error::NotFound)
        ));
        create_session(&pool, "a1", anna, "c", 1000, 9000)
            .await
            .unwrap();
        create_session(&pool, "a2", anna, "c", 1000, 9000)
            .await
            .unwrap();
        create_session(&pool, "w1", wall, "c", 1000, 9000)
            .await
            .unwrap();
        touch_session(&pool, "a2", 1500).await.unwrap();
        let a2 = find_session(&pool, "a2", 1600).await.unwrap().unwrap();
        assert_eq!((a2.last_seen_at, a2.stay_signed_in), (1500, false));

        // 30 minutes after 1000: a1 is idle, a2 was active later, the wall display is exempt.
        let now = 1000 + 1800;
        let sessions =
            |infos: Vec<UserInfo>| -> Vec<i64> { infos.iter().map(|i| i.sessions).collect() };
        assert_eq!(
            sessions(list_user_infos(&pool, now, Some(1800)).await.unwrap()),
            [1, 1]
        );
        assert_eq!(
            sessions(list_user_infos(&pool, now, None).await.unwrap()),
            [2, 1]
        );
        purge_expired_sessions(&pool, now, Some(1800))
            .await
            .unwrap();
        assert!(find_session(&pool, "a1", now).await.unwrap().is_none());
        assert!(find_session(&pool, "a2", now).await.unwrap().is_some());
        assert!(find_session(&pool, "w1", now).await.unwrap().is_some());
        // The lifetime applies to everyone.
        purge_expired_sessions(&pool, 9000, Some(1800))
            .await
            .unwrap();
        assert!(find_session(&pool, "w1", 8999).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn set_password_hash_reports_missing_user() {
        let (pool, _dir) = pool().await;
        create_user(&pool, "anna", "old", Role::Viewer, None)
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
        let a = create_user(&pool, "anna", "h", Role::Admin, None)
            .await
            .unwrap();
        let b = create_user(&pool, "bert", "h", Role::Viewer, None)
            .await
            .unwrap();
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
        let infos = list_user_infos(&pool, 0, None).await.unwrap();
        assert_eq!(
            (infos.len(), infos[0].role.as_str(), infos[0].sessions),
            (1, "admin", 0)
        );
    }

    #[tokio::test]
    async fn changing_a_password_ends_all_sessions_but_the_kept_one() {
        let (pool, _dir) = pool().await;
        let a = create_user(&pool, "anna", "old", Role::Admin, None)
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
