//! Persistence of the audit log (see [`crate::audit`]): table `audit_log`.

use sqlx::SqlitePool;

use crate::audit::{Action, Kind};
use crate::error::Result;
use crate::i18n::Msg;

/// What is written for one entry.
#[derive(Debug, Clone)]
pub struct NewEntry<'a> {
    pub ts_ms: i64,
    pub user: &'a str,
    pub address: Option<String>,
    pub action: Action,
    pub kind: Kind,
    pub object_id: Option<i64>,
    pub object_name: &'a str,
    pub detail: &'a Msg,
}

pub async fn insert(pool: &SqlitePool, e: &NewEntry<'_>) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (ts_ms, user_name, address, action, kind, object_id, object_name, detail) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(e.ts_ms)
    .bind(e.user)
    .bind(&e.address)
    .bind(e.action.as_str())
    .bind(e.kind.as_str())
    .bind(e.object_id)
    .bind(e.object_name)
    .bind(e.detail.encode())
    .execute(pool)
    .await?;
    Ok(())
}

/// One stored entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: i64,
    pub ts_ms: i64,
    pub user: String,
    pub address: Option<String>,
    pub action: Action,
    pub kind: Kind,
    pub object_id: Option<i64>,
    pub object_name: String,
    pub detail: Msg,
}

type Row = (
    i64,
    i64,
    String,
    Option<String>,
    String,
    String,
    Option<i64>,
    String,
    String,
);

/// The newest `limit` entries, newest first (entries of unknown actions or kinds are skipped).
pub async fn list(pool: &SqlitePool, limit: u32) -> Result<Vec<Entry>> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, ts_ms, user_name, address, action, kind, object_id, object_name, detail \
         FROM audit_log ORDER BY ts_ms DESC, id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(
            |(id, ts_ms, user, address, action, kind, object_id, object_name, detail)| {
                Some(Entry {
                    id,
                    ts_ms,
                    user,
                    address,
                    action: Action::parse(&action)?,
                    kind: Kind::parse(&kind)?,
                    object_id,
                    object_name,
                    detail: Msg::decode(&detail),
                })
            },
        )
        .collect())
}

/// Deletes entries older than `before_ms`.
pub async fn apply_retention(pool: &SqlitePool, before_ms: i64) -> Result<u64> {
    Ok(sqlx::query("DELETE FROM audit_log WHERE ts_ms < ?")
        .bind(before_ms)
        .execute(pool)
        .await?
        .rows_affected())
}
