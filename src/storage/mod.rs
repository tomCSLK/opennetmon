//! SQLite storage: connection pool (WAL mode) and migrations.

pub mod availability;
pub mod credentials;
pub mod devices;
pub mod groups;
pub mod mail;
pub mod measurements;
pub mod mibs;
pub mod pauses;
pub mod schedules;
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
    current_sensors_table(pool).await?;
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
                           retries, created_at, updated_at, position, template_id, kind, \
                           paused, pause_until_ms, pause_reason, dependency, dependency_sensor_id, \
                           dependency_delay_secs";
    rebuild_without_foreign_keys(pool, "devices", |tx| {
        Box::pin(async move {
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
                    template_id   INTEGER REFERENCES device_templates (id) ON DELETE SET NULL,
                    kind          TEXT NOT NULL DEFAULT 'generic',
                    paused        INTEGER NOT NULL DEFAULT 0 CHECK (paused IN (0, 1)),
                    pause_until_ms INTEGER,
                    pause_reason  TEXT NOT NULL DEFAULT '',
                    -- dependencies (migration 0019)
                    dependency    TEXT NOT NULL DEFAULT 'parent' CHECK (dependency IN ('parent', 'sensor', 'none')),
                    dependency_sensor_id INTEGER REFERENCES sensors (id) ON DELETE SET NULL,
                    dependency_delay_secs INTEGER CHECK (dependency_delay_secs IS NULL OR dependency_delay_secs BETWEEN 0 AND 3600)
                ) STRICT",
            )
            .execute(&mut **tx)
            .await?;
            sqlx::query(&format!(
                "INSERT INTO devices_new ({COLUMNS}) SELECT {COLUMNS} FROM devices"
            ))
            .execute(&mut **tx)
            .await?;
            for statement in [
                "DROP TABLE devices",
                "ALTER TABLE devices_new RENAME TO devices",
                "CREATE INDEX devices_group_id ON devices (group_id)",
                "CREATE INDEX devices_profile_id ON devices (profile_id)",
            ] {
                sqlx::query(statement).execute(&mut **tx).await?;
            }
            Ok(())
        })
    })
    .await
}

/// The `sensors` table of migration 0005 has a check on `kind` that later sensor types do not fit:
/// sensors of the machine opennetmon runs on (kind `local`, measured value in `target`, e.g.
/// `disk:/var`) and service sensors (`http`, `tcp`, `dns`, `tls`, settings as JSON in `params`).
/// Like [`allow_devices_without_profile`], the check only changes by rebuilding the table outside a
/// migration transaction; it runs once per schema change (detected by the newest column) and copies
/// the columns the old table has. Later migrations that add columns to `sensors` must add them here
/// as well, and a new sensor type needs a new newest column (or another check) to trigger it.
async fn current_sensors_table(pool: &SqlitePool) -> Result<()> {
    let existing: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('sensors')")
        .fetch_all(pool)
        .await?;
    if existing.iter().any(|c| c == "params") {
        return Ok(());
    }
    const COLUMNS: [&str; 20] = [
        "id",
        "device_id",
        "name",
        "kind",
        "if_index",
        "oid",
        "oid_mode",
        "scale",
        "enabled",
        "created_at",
        "unit_label",
        "position",
        "pause_until_ms",
        "pause_reason",
        "target",
        "interval_secs",
        "dependency",
        "dependency_sensor_id",
        "dependency_delay_secs",
        "dependency_master",
    ];
    let copied = COLUMNS
        .iter()
        .filter(|c| existing.iter().any(|e| e == *c))
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    rebuild_without_foreign_keys(pool, "sensors", |tx| {
        Box::pin(async move {
            sqlx::query(
                "CREATE TABLE sensors_new (
                    id             INTEGER PRIMARY KEY,
                    device_id      INTEGER NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
                    name           TEXT NOT NULL COLLATE NOCASE,
                    kind           TEXT NOT NULL CHECK (kind IN ('ping', 'uptime', 'if_traffic', 'if_status', 'oid', 'local', 'http', 'tcp', 'dns', 'tls')),
                    if_index       INTEGER,
                    oid            TEXT,
                    oid_mode       TEXT CHECK (oid_mode IN ('gauge', 'counter')),
                    scale          REAL,
                    enabled        INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
                    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                    unit_label     TEXT,
                    position       INTEGER CHECK (position IS NULL OR position > 0),
                    pause_until_ms INTEGER,
                    pause_reason   TEXT NOT NULL DEFAULT '',
                    -- what a local sensor measures, e.g. 'cpu' or 'disk:/var'
                    target         TEXT,
                    -- settings of a service sensor (JSON, checked by the program)
                    params         TEXT CHECK (params IS NULL OR json_valid(params)),
                    -- own polling interval (migration 0017); NULL = the device's
                    interval_secs  INTEGER CHECK (interval_secs IS NULL OR interval_secs BETWEEN 10 AND 3600),
                    -- dependencies (migration 0019)
                    dependency     TEXT NOT NULL DEFAULT 'parent' CHECK (dependency IN ('parent', 'sensor', 'none')),
                    dependency_sensor_id INTEGER REFERENCES sensors (id) ON DELETE SET NULL,
                    dependency_delay_secs INTEGER CHECK (dependency_delay_secs IS NULL OR dependency_delay_secs BETWEEN 0 AND 3600),
                    dependency_master INTEGER NOT NULL DEFAULT 0 CHECK (dependency_master IN (0, 1)),
                    UNIQUE (device_id, name),
                    -- Exactly the columns that belong to the kind are set.
                    CHECK (
                        CASE kind
                            WHEN 'if_traffic' THEN if_index IS NOT NULL AND oid IS NULL AND oid_mode IS NULL AND scale IS NULL AND target IS NULL AND params IS NULL
                            WHEN 'if_status'  THEN if_index IS NOT NULL AND oid IS NULL AND oid_mode IS NULL AND scale IS NULL AND target IS NULL AND params IS NULL
                            WHEN 'oid'        THEN if_index IS NULL AND oid IS NOT NULL AND oid_mode IS NOT NULL AND scale IS NOT NULL AND target IS NULL AND params IS NULL
                            WHEN 'local'      THEN if_index IS NULL AND oid IS NULL AND oid_mode IS NULL AND scale IS NULL AND target IS NOT NULL AND params IS NULL
                            WHEN 'http'       THEN if_index IS NULL AND oid IS NULL AND oid_mode IS NULL AND scale IS NULL AND target IS NULL AND params IS NOT NULL
                            WHEN 'tcp'        THEN if_index IS NULL AND oid IS NULL AND oid_mode IS NULL AND scale IS NULL AND target IS NULL AND params IS NOT NULL
                            WHEN 'dns'        THEN if_index IS NULL AND oid IS NULL AND oid_mode IS NULL AND scale IS NULL AND target IS NULL AND params IS NOT NULL
                            WHEN 'tls'        THEN if_index IS NULL AND oid IS NULL AND oid_mode IS NULL AND scale IS NULL AND target IS NULL AND params IS NOT NULL
                            ELSE if_index IS NULL AND oid IS NULL AND oid_mode IS NULL AND scale IS NULL AND target IS NULL AND params IS NULL
                        END
                    )
                ) STRICT",
            )
            .execute(&mut **tx)
            .await?;
            sqlx::query(&format!(
                "INSERT INTO sensors_new ({copied}) SELECT {copied} FROM sensors"
            ))
            .execute(&mut **tx)
            .await?;
            for statement in [
                "DROP TABLE sensors",
                "ALTER TABLE sensors_new RENAME TO sensors",
                "CREATE INDEX sensors_device_id ON sensors (device_id)",
            ] {
                sqlx::query(statement).execute(&mut **tx).await?;
            }
            Ok(())
        })
    })
    .await
}

/// Runs `rebuild` in a transaction with foreign keys switched off (so dropping the old table
/// does not cascade), then checks that every foreign key still holds before committing.
async fn rebuild_without_foreign_keys<F>(pool: &SqlitePool, table: &str, rebuild: F) -> Result<()>
where
    F: for<'t> FnOnce(
        &'t mut sqlx::Transaction<'_, sqlx::Sqlite>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<()>> + Send + 't>,
    >,
{
    let mut conn = pool.acquire().await?;
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *conn)
        .await?;
    let rebuilt = async {
        let mut tx = sqlx::Connection::begin(&mut *conn).await?;
        rebuild(&mut tx).await?;
        let broken = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut *tx)
            .await?;
        if !broken.is_empty() {
            return Err(crate::error::Error::Internal(format!(
                "rebuilding {table} would break {} foreign key(s); nothing changed",
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
    async fn the_sensors_table_is_rebuilt_once_for_local_sensors_without_losing_data() {
        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("old.db")).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for statement in [
            "INSERT INTO credential_profiles (name, version, community_enc) VALUES ('p', 'v2c', x'01')",
            "INSERT INTO devices (name, host, profile_id) VALUES ('srv', '127.0.0.1', 1)",
            "INSERT INTO sensors (device_id, name, kind, position, unit_label, pause_reason) \
             VALUES (1, 'Ping', 'ping', 2, NULL, 'Umbau')",
            "INSERT INTO channels (sensor_id, key, unit, warn_above) VALUES (1, 'rtt', 'ms', 50)",
            "INSERT INTO samples (channel_id, ts, value) VALUES (1, 1000, 2.5)",
            "INSERT INTO sensor_status (sensor_id, status, message, since_ms, last_poll_ms, consecutive_failures) \
             VALUES (1, 'up', '', 1000, 1000, 0)",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        let local =
            "INSERT INTO sensors (device_id, name, kind, target) VALUES (1, 'CPU', 'local', 'cpu')";
        assert!(sqlx::query(local).execute(&pool).await.is_err());

        migrate(&pool).await.unwrap();
        migrate(&pool).await.unwrap(); // a second start changes nothing
        let kept: (String, Option<i64>, String) =
            sqlx::query_as("SELECT name, position, pause_reason FROM sensors WHERE id = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(kept, ("Ping".to_owned(), Some(2), "Umbau".to_owned()));
        let rows: (i64, i64, Option<f64>) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM samples), (SELECT COUNT(*) FROM sensor_status), \
             (SELECT warn_above FROM channels WHERE id = 1)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(rows, (1, 1, Some(50.0)));
        sqlx::query(local).execute(&pool).await.unwrap();
        assert!(
            sqlx::query("INSERT INTO sensors (device_id, name, kind) VALUES (1, 'x', 'local')")
                .execute(&pool)
                .await
                .is_err(),
            "a local sensor needs its target"
        );
        sqlx::query("DELETE FROM devices WHERE id = 1")
            .execute(&pool)
            .await
            .unwrap();
        let left: (i64, i64) =
            sqlx::query_as("SELECT (SELECT COUNT(*) FROM sensors), (SELECT COUNT(*) FROM samples)")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(left, (0, 0), "cascade");
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
