//! SQLite storage: connection pool (WAL mode) and migrations.

pub mod credentials;
pub mod devices;
pub mod groups;
pub mod mail;
pub mod measurements;
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

/// Cheap liveness check for the health endpoint.
pub async fn ping(pool: &SqlitePool) -> Result<()> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
