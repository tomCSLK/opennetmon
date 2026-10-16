//! SQLite storage: connection pool (WAL mode) and migrations.

pub mod credentials;
pub mod devices;
pub mod groups;
pub mod mail;
pub mod measurements;
pub mod mibs;
pub mod sensors;
pub mod status;
pub mod users;

use std::path::Path;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use crate::error::Result;

/// Opens (and creates, if missing) the database in WAL mode.
pub async fn connect(path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        tokio::fs::create_dir_all(parent).await?;
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;
    Ok(pool)
}

/// Applies all pending migrations from `migrations/` (embedded at compile time).
pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

/// Writes a consistent copy of the database to `target` (SQLite `VACUUM INTO`); safe while the server runs.
/// The file is created with mode 0600 and an existing non-empty file is never overwritten.
/// It contains the encrypted SNMP/SMTP secrets but not the key file, which is backed up separately.
pub async fn backup_to(pool: &SqlitePool, target: &Path) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    let target = std::path::absolute(target)?;
    let text = target.to_str().ok_or_else(|| {
        crate::error::Error::Invalid("Der Pfad der Sicherung ist kein gültiges UTF-8.".to_owned())
    })?;
    // SQLite accepts a missing or an empty target; create it ourselves so the mode is right from the start.
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&target)
    {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::metadata(&target)?.len() > 0 {
                return Err(crate::error::Error::Invalid(format!(
                    "{} existiert bereits; eine vorhandene Sicherung wird nie überschrieben.",
                    target.display()
                )));
            }
        }
        Err(e) => return Err(e.into()),
    }
    let done = sqlx::query("VACUUM INTO ?").bind(text).execute(pool).await;
    if done.is_err() {
        let _ = std::fs::remove_file(&target); // do not leave a half-made backup behind
    }
    done?;
    Ok(())
}

/// Where manual positions live: table and the column that scopes them (siblings share its value).
#[derive(Debug, Clone, Copy)]
pub(crate) enum OrderScope {
    /// Groups below a parent (`None` = top level).
    Groups(Option<i64>),
    /// Devices in a group (`None` = without group).
    Devices(Option<i64>),
    /// Sensors of a device.
    Sensors(i64),
}

impl OrderScope {
    fn sql(self) -> (&'static str, &'static str, Option<i64>) {
        match self {
            Self::Groups(parent) => ("groups", "parent_id", parent),
            Self::Devices(group) => ("devices", "group_id", group),
            Self::Sensors(device) => ("sensors", "device_id", Some(device)),
        }
    }
}

/// Stores `ids` as the manual order (1, 2, …) of their scope; every other item of the scope goes back
/// to automatic order. An empty list resets the whole scope. Ids outside the scope are ignored.
pub(crate) async fn write_positions(
    pool: &SqlitePool,
    scope: OrderScope,
    ids: &[i64],
) -> Result<()> {
    let (table, column, value) = scope.sql();
    let mut tx = pool.begin().await?;
    sqlx::query(&format!(
        "UPDATE {table} SET position = NULL WHERE {column} IS ?"
    ))
    .bind(value)
    .execute(&mut *tx)
    .await?;
    let update = format!("UPDATE {table} SET position = ? WHERE id = ? AND {column} IS ?");
    for (i, id) in ids.iter().enumerate() {
        sqlx::query(&update)
            .bind(i64::try_from(i).unwrap_or(i64::MAX - 1) + 1)
            .bind(id)
            .bind(value)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Cheap liveness check for the health endpoint.
pub async fn ping(pool: &SqlitePool) -> Result<()> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn backup_is_a_consistent_private_copy_and_never_overwrites() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("live.db")).await.unwrap();
        migrate(&pool).await.unwrap();
        sqlx::query("INSERT INTO groups (name) VALUES ('Keller')")
            .execute(&pool)
            .await
            .unwrap();

        let target = dir.path().join("backup.db");
        backup_to(&pool, &target).await.unwrap();
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let copy = connect(&target).await.unwrap();
        let names: Vec<String> = sqlx::query_scalar("SELECT name FROM groups")
            .fetch_all(&copy)
            .await
            .unwrap();
        assert_eq!(names, ["Keller"]);
        copy.close().await;

        // A second run must not touch the first backup.
        sqlx::query("INSERT INTO groups (name) VALUES ('Büro')")
            .execute(&pool)
            .await
            .unwrap();
        let err = backup_to(&pool, &target).await.unwrap_err();
        assert!(matches!(err, crate::error::Error::Invalid(_)), "{err:?}");
        let copy = connect(&target).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM groups")
            .fetch_one(&copy)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn a_failed_backup_leaves_no_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("live.db")).await.unwrap();
        let missing_dir = dir.path().join("nope").join("backup.db");
        assert!(backup_to(&pool, &missing_dir).await.is_err());
        assert!(!missing_dir.exists());
    }

    #[tokio::test]
    async fn connect_enables_wal_and_migrations_apply() {
        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("nested").join("test.db"))
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        // Running again must be a no-op.
        migrate(&pool).await.unwrap();

        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(mode, "wal");

        let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(fk, 1);

        let created: String = sqlx::query_scalar("SELECT value FROM meta WHERE key = 'created_at'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(created.ends_with('Z'));
        ping(&pool).await.unwrap();
    }
}
