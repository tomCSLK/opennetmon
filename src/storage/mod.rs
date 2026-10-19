//! SQLite storage: connection pool (WAL mode) and migrations.

pub mod credentials;
pub mod devices;
pub mod groups;
pub mod mail;
pub mod measurements;
pub mod mibs;
pub mod sensors;
pub mod status;
pub mod templates;
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

/// Applies all pending migrations from `migrations/` (embedded at compile time) and the schema
/// changes that cannot run inside a migration transaction.
pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    allow_devices_without_profile(pool).await?;
    Ok(())
}

/// Migration 0004 made `devices.profile_id` NOT NULL; ping-only devices need NULL ("kein SNMP").
/// SQLite drops the constraint only by rebuilding the table, which needs foreign keys switched off
/// (otherwise dropping the old table would cascade to all sensors) – impossible inside sqlx's
/// migration transaction. So it runs here, once: afterwards the column allows NULL and this is a no-op.
async fn allow_devices_without_profile(pool: &SqlitePool) -> Result<()> {
    let not_null: i64 = sqlx::query_scalar(
        "SELECT \"notnull\" FROM pragma_table_info('devices') WHERE name = 'profile_id'",
    )
    .fetch_one(pool)
    .await?;
    if not_null == 0 {
        return Ok(());
    }
    const COLUMNS: &str = "id, name, host, port, group_id, profile_id, interval_secs, timeout_secs, \
                           retries, created_at, updated_at, position, template_id";
    let mut conn = pool.acquire().await?;
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *conn)
        .await?;
    let rebuilt = async {
        let mut tx = sqlx::Connection::begin(&mut *conn).await?;
        sqlx::query(
            "CREATE TABLE devices_new (
                id            INTEGER PRIMARY KEY,
                name          TEXT NOT NULL COLLATE NOCASE UNIQUE,
                host          TEXT NOT NULL,
                port          INTEGER NOT NULL DEFAULT 161 CHECK (port BETWEEN 1 AND 65535),
                group_id      INTEGER REFERENCES groups (id) ON DELETE RESTRICT,
                -- NULL = no SNMP (ping only)
                profile_id    INTEGER REFERENCES credential_profiles (id) ON DELETE RESTRICT,
                interval_secs INTEGER NOT NULL DEFAULT 60 CHECK (interval_secs BETWEEN 10 AND 3600),
                timeout_secs  INTEGER NOT NULL DEFAULT 5 CHECK (timeout_secs BETWEEN 1 AND 60),
                retries       INTEGER NOT NULL DEFAULT 1 CHECK (retries BETWEEN 0 AND 5),
                created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                position      INTEGER CHECK (position IS NULL OR position > 0),
                template_id   INTEGER REFERENCES device_templates (id) ON DELETE SET NULL
            ) STRICT",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(&format!(
            "INSERT INTO devices_new ({COLUMNS}) SELECT {COLUMNS} FROM devices"
        ))
        .execute(&mut *tx)
        .await?;
        for statement in [
            "DROP TABLE devices",
            "ALTER TABLE devices_new RENAME TO devices",
            "CREATE INDEX devices_group_id ON devices (group_id)",
            "CREATE INDEX devices_profile_id ON devices (profile_id)",
        ] {
            sqlx::query(statement).execute(&mut *tx).await?;
        }
        let broken = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut *tx)
            .await?;
        if !broken.is_empty() {
            return Err(crate::error::Error::Internal(format!(
                "rebuilding devices would break {} foreign key(s); nothing changed",
                broken.len()
            )));
        }
        tx.commit().await?;
        Ok(())
    }
    .await;
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *conn)
        .await?;
    rebuilt
}

/// Writes a consistent copy of the database to `target` (SQLite `VACUUM INTO`); safe while the server runs.
/// The file is created with mode 0600 and an existing non-empty file is never overwritten.
/// It contains the encrypted SNMP/SMTP secrets but not the key file, which is backed up separately.
pub async fn backup_to(pool: &SqlitePool, target: &Path) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    let target = std::path::absolute(target)?;
    let text = target.to_str().ok_or_else(|| {
        crate::error::Error::invalid(crate::i18n::Msg::new("backup-path-not-utf8"))
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
                return Err(crate::error::Error::invalid(
                    crate::i18n::Msg::new("backup-exists")
                        .with("path", target.display().to_string()),
                ));
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

    #[tokio::test]
    async fn the_devices_table_is_rebuilt_once_without_losing_data() {
        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("old.db")).await.unwrap();
        // The state before the rebuild: all migrations, profile still mandatory.
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for statement in [
            "INSERT INTO credential_profiles (name, version, community_enc) VALUES ('p', 'v2c', x'01')",
            "INSERT INTO groups (name) VALUES ('Keller')",
            "INSERT INTO devices (name, host, group_id, profile_id, position) VALUES ('sw', '192.0.2.1', 1, 1, 3)",
            "INSERT INTO sensors (device_id, name, kind) VALUES (1, 'Ping', 'ping')",
            "INSERT INTO channels (sensor_id, key, unit) VALUES (1, 'rtt', 'ms')",
            "INSERT INTO samples (channel_id, ts, value) VALUES (1, 1000, 2.5)",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        assert!(
            sqlx::query(
                "INSERT INTO devices (name, host, profile_id) VALUES ('x', '192.0.2.2', NULL)"
            )
            .execute(&pool)
            .await
            .is_err()
        );

        migrate(&pool).await.unwrap();
        migrate(&pool).await.unwrap(); // a second start changes nothing
        let count = |table: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
                    .fetch_one(&pool)
                    .await
                    .unwrap()
            }
        };
        assert_eq!((count("sensors").await, count("samples").await), (1, 1));
        let kept: (String, Option<i64>, Option<i64>) =
            sqlx::query_as("SELECT name, group_id, position FROM devices WHERE id = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(kept, ("sw".to_owned(), Some(1), Some(3)));
        sqlx::query(
            "INSERT INTO devices (name, host, profile_id) VALUES ('Drucker', '192.0.2.2', NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Foreign keys still work in both directions.
        assert!(
            sqlx::query("DELETE FROM credential_profiles WHERE id = 1")
                .execute(&pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("DELETE FROM groups WHERE id = 1")
                .execute(&pool)
                .await
                .is_err()
        );
        sqlx::query("DELETE FROM devices WHERE id = 1")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            (count("sensors").await, count("samples").await),
            (0, 0),
            "cascade"
        );
    }
}
