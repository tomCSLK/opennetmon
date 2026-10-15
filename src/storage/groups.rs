//! Persistence of hierarchical device groups.

use sqlx::{FromRow, SqlitePool};

use crate::devices::{
    Group, MAX_GROUP_DEPTH, depth_of, subtree_height, subtree_ids, validate_name,
};
use crate::error::{Error, Result};

#[derive(FromRow)]
struct Row {
    id: i64,
    name: String,
    parent_id: Option<i64>,
}

impl From<Row> for Group {
    fn from(r: Row) -> Self {
        Self {
            id: r.id,
            name: r.name,
            parent_id: r.parent_id,
        }
    }
}

pub async fn list_groups(pool: &SqlitePool) -> Result<Vec<Group>> {
    Ok(
        sqlx::query_as::<_, Row>("SELECT id, name, parent_id FROM groups ORDER BY name")
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(Group::from)
            .collect(),
    )
}

pub async fn get_group(pool: &SqlitePool, id: i64) -> Result<Option<Group>> {
    Ok(
        sqlx::query_as::<_, Row>("SELECT id, name, parent_id FROM groups WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .map(Group::from),
    )
}

fn invalid(message: &str) -> Error {
    Error::Invalid(message.to_owned())
}

fn name_taken() -> Error {
    Error::Conflict("In dieser Ebene gibt es bereits eine Gruppe mit diesem Namen.".to_owned())
}

/// Creates a group below `parent_id` (or at the top level).
pub async fn create_group(pool: &SqlitePool, name: &str, parent_id: Option<i64>) -> Result<i64> {
    let name = validate_name(name).map_err(invalid)?;
    let groups = list_groups(pool).await?;
    if let Some(parent) = parent_id
        && !groups.iter().any(|g| g.id == parent)
    {
        return Err(invalid("Die übergeordnete Gruppe existiert nicht."));
    }
    if depth_of(&groups, parent_id) + 1 > MAX_GROUP_DEPTH {
        return Err(invalid(
            "Die Gruppen sind höchstens 8 Ebenen tief verschachtelbar.",
        ));
    }
    let result = sqlx::query("INSERT INTO groups (name, parent_id) VALUES (?, ?)")
        .bind(name)
        .bind(parent_id)
        .execute(pool)
        .await;
    match result {
        Ok(done) => Ok(done.last_insert_rowid()),
        Err(e)
            if e.as_database_error()
                .is_some_and(|d| d.is_unique_violation()) =>
        {
            Err(name_taken())
        }
        Err(e) => Err(e.into()),
    }
}

/// Renames and/or moves a group. A group cannot be moved below itself or one of its descendants.
pub async fn update_group(
    pool: &SqlitePool,
    id: i64,
    name: &str,
    parent_id: Option<i64>,
) -> Result<()> {
    let name = validate_name(name).map_err(invalid)?;
    let groups = list_groups(pool).await?;
    if !groups.iter().any(|g| g.id == id) {
        return Err(Error::NotFound);
    }
    if let Some(parent) = parent_id {
        if !groups.iter().any(|g| g.id == parent) {
            return Err(invalid("Die übergeordnete Gruppe existiert nicht."));
        }
        if subtree_ids(&groups, id).contains(&parent) {
            return Err(invalid(
                "Eine Gruppe kann nicht in sich selbst oder in eine ihrer Untergruppen verschoben werden.",
            ));
        }
    }
    if depth_of(&groups, parent_id) + 1 + subtree_height(&groups, id) > MAX_GROUP_DEPTH {
        return Err(invalid(
            "Die Gruppen sind höchstens 8 Ebenen tief verschachtelbar.",
        ));
    }
    let result = sqlx::query("UPDATE groups SET name = ?, parent_id = ? WHERE id = ?")
        .bind(name)
        .bind(parent_id)
        .bind(id)
        .execute(pool)
        .await;
    match result {
        Ok(_) => Ok(()),
        Err(e)
            if e.as_database_error()
                .is_some_and(|d| d.is_unique_violation()) =>
        {
            Err(name_taken())
        }
        Err(e) => Err(e.into()),
    }
}

/// Deletes an empty group (no subgroups, no devices).
pub async fn delete_group(pool: &SqlitePool, id: i64) -> Result<()> {
    let children: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM groups WHERE parent_id = ?")
        .bind(id)
        .fetch_one(pool)
        .await?;
    if children > 0 {
        return Err(Error::Conflict(
            "Die Gruppe enthält noch Untergruppen.".to_owned(),
        ));
    }
    let members: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM devices WHERE group_id = ?")
        .bind(id)
        .fetch_one(pool)
        .await?;
    if members > 0 {
        return Err(Error::Conflict(
            "Die Gruppe enthält noch Geräte.".to_owned(),
        ));
    }
    let done = sqlx::query("DELETE FROM groups WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

/// Number of devices directly in each group, for the overview.
pub async fn device_counts(pool: &SqlitePool) -> Result<Vec<(i64, i64)>> {
    Ok(sqlx::query_as(
        "SELECT group_id, COUNT(*) FROM devices WHERE group_id IS NOT NULL GROUP BY group_id",
    )
    .fetch_all(pool)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

    async fn pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pool = storage::connect(&dir.path().join("g.db")).await.unwrap();
        storage::migrate(&pool).await.unwrap();
        (pool, dir)
    }

    #[tokio::test]
    async fn create_nest_and_list() {
        let (pool, _dir) = pool().await;
        let home = create_group(&pool, " Zuhause ", None).await.unwrap();
        let cellar = create_group(&pool, "Keller", Some(home)).await.unwrap();
        let groups = list_groups(&pool).await.unwrap();
        assert_eq!(groups.len(), 2);
        let cellar = groups.iter().find(|g| g.id == cellar).unwrap();
        assert_eq!(
            (cellar.name.as_str(), cellar.parent_id),
            ("Keller", Some(home))
        );
        assert_eq!(
            groups.iter().find(|g| g.id == home).unwrap().name,
            "Zuhause"
        );
    }

    #[tokio::test]
    async fn names_are_unique_per_level_only() {
        let (pool, _dir) = pool().await;
        create_group(&pool, "Buero", None).await.unwrap();
        // Same name at the same (top) level, any ASCII case: rejected.
        assert!(matches!(
            create_group(&pool, "Buero", None).await,
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            create_group(&pool, "BUERO", None).await,
            Err(Error::Conflict(_))
        ));
        // Same name below another parent: fine.
        let b = create_group(&pool, "Haus", None).await.unwrap();
        create_group(&pool, "Buero", Some(b)).await.unwrap();
        assert!(matches!(
            create_group(&pool, "buero", Some(b)).await,
            Err(Error::Conflict(_))
        ));
    }

    #[tokio::test]
    async fn invalid_input_is_rejected() {
        let (pool, _dir) = pool().await;
        assert!(matches!(
            create_group(&pool, "  ", None).await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            create_group(&pool, "x", Some(99)).await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            update_group(&pool, 99, "x", None).await,
            Err(Error::NotFound)
        ));
    }

    #[tokio::test]
    async fn moving_is_checked_against_cycles() {
        let (pool, _dir) = pool().await;
        let a = create_group(&pool, "a", None).await.unwrap();
        let b = create_group(&pool, "b", Some(a)).await.unwrap();
        let c = create_group(&pool, "c", Some(b)).await.unwrap();
        assert!(matches!(
            update_group(&pool, a, "a", Some(a)).await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            update_group(&pool, a, "a", Some(c)).await,
            Err(Error::Invalid(_))
        ));
        // Moving c to the top level works, so does renaming.
        update_group(&pool, c, "c2", None).await.unwrap();
        let moved = get_group(&pool, c).await.unwrap().unwrap();
        assert_eq!((moved.name.as_str(), moved.parent_id), ("c2", None));
    }

    #[tokio::test]
    async fn depth_is_limited() {
        let (pool, _dir) = pool().await;
        let mut parent = None;
        for level in 1..=MAX_GROUP_DEPTH {
            parent = Some(
                create_group(&pool, &format!("l{level}"), parent)
                    .await
                    .unwrap(),
            );
        }
        assert!(matches!(
            create_group(&pool, "zu tief", parent).await,
            Err(Error::Invalid(_))
        ));
        // Moving a deep subtree below a deep parent is checked as well.
        let top = create_group(&pool, "andere", None).await.unwrap();
        let deep = create_group(&pool, "tief", None).await.unwrap();
        create_group(&pool, "tief-kind", Some(deep)).await.unwrap();
        assert!(update_group(&pool, deep, "tief", parent).await.is_err());
        update_group(&pool, deep, "tief", Some(top)).await.unwrap();
    }

    #[tokio::test]
    async fn only_empty_groups_can_be_deleted() {
        let (pool, _dir) = pool().await;
        let parent = create_group(&pool, "p", None).await.unwrap();
        let child = create_group(&pool, "c", Some(parent)).await.unwrap();
        assert!(matches!(
            delete_group(&pool, parent).await,
            Err(Error::Conflict(_))
        ));
        delete_group(&pool, child).await.unwrap();
        delete_group(&pool, parent).await.unwrap();
        assert!(matches!(
            delete_group(&pool, parent).await,
            Err(Error::NotFound)
        ));
    }
}
