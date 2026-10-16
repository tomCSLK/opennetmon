//! Persistence of uploaded MIB files. The parsed index is built from these texts in memory.

use sqlx::{FromRow, SqlitePool};

use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct MibFile {
    pub id: i64,
    pub file_name: String,
    /// Module names defined by the file, separated by spaces.
    pub modules: String,
    pub size: i64,
    pub uploaded_at: String,
}

/// A validated upload: file name, the module names it defines and its text.
pub struct NewMibFile {
    pub file_name: String,
    pub modules: Vec<String>,
    pub content: String,
}

pub async fn list_files(pool: &SqlitePool) -> Result<Vec<MibFile>> {
    Ok(sqlx::query_as::<_, MibFile>(
        "SELECT id, file_name, modules, size, uploaded_at FROM mib_files ORDER BY file_name COLLATE NOCASE, id",
    )
    .fetch_all(pool)
    .await?)
}

/// All file texts, oldest first (later uploads override earlier ones in the index).
pub async fn all_contents(pool: &SqlitePool) -> Result<Vec<(i64, String)>> {
    Ok(
        sqlx::query_as("SELECT id, content FROM mib_files ORDER BY id")
            .fetch_all(pool)
            .await?,
    )
}

/// Stores files in one transaction. A stored file that defines one of the same modules is
/// replaced, so uploading a newer version of a MIB simply supersedes the old one.
/// Returns the number of replaced files.
pub async fn save_files(pool: &SqlitePool, files: &[NewMibFile]) -> Result<u64> {
    let mut tx = pool.begin().await?;
    let existing: Vec<(i64, String)> = sqlx::query_as("SELECT id, modules FROM mib_files")
        .fetch_all(&mut *tx)
        .await?;
    let mut replaced = 0;
    for (id, modules) in existing {
        let clashes = modules
            .split_whitespace()
            .any(|m| files.iter().any(|f| f.modules.iter().any(|n| n == m)));
        if clashes {
            sqlx::query("DELETE FROM mib_files WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            replaced += 1;
        }
    }
    for f in files {
        sqlx::query(
            "INSERT INTO mib_files (file_name, modules, content, size) VALUES (?, ?, ?, ?)",
        )
        .bind(&f.file_name)
        .bind(f.modules.join(" "))
        .bind(&f.content)
        .bind(i64::try_from(f.content.len()).unwrap_or(i64::MAX))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(replaced)
}

pub async fn delete_file(pool: &SqlitePool, id: i64) -> Result<bool> {
    let done = sqlx::query("DELETE FROM mib_files WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{connect, migrate};

    fn file(name: &str, modules: &[&str], content: &str) -> NewMibFile {
        NewMibFile {
            file_name: name.to_owned(),
            modules: modules.iter().map(|m| (*m).to_owned()).collect(),
            content: content.to_owned(),
        }
    }

    #[tokio::test]
    async fn files_are_stored_listed_replaced_by_module_and_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("m.db")).await.unwrap();
        migrate(&pool).await.unwrap();

        let replaced = save_files(
            &pool,
            &[
                file("b.mib", &["B-MIB"], "b1"),
                file("a.mib", &["A-MIB", "A2-MIB"], "a1"),
            ],
        )
        .await
        .unwrap();
        assert_eq!(replaced, 0);
        let listed = list_files(&pool).await.unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|f| f.file_name.as_str())
                .collect::<Vec<_>>(),
            ["a.mib", "b.mib"]
        );
        assert_eq!(listed[0].modules, "A-MIB A2-MIB");
        assert_eq!(listed[0].size, 2);

        // A new file defining A2-MIB replaces the old a.mib.
        let replaced = save_files(&pool, &[file("a-new.mib", &["A2-MIB"], "a2")])
            .await
            .unwrap();
        assert_eq!(replaced, 1);
        let contents = all_contents(&pool).await.unwrap();
        assert_eq!(
            contents.iter().map(|(_, c)| c.as_str()).collect::<Vec<_>>(),
            ["b1", "a2"]
        );

        let id = list_files(&pool).await.unwrap()[0].id;
        assert!(delete_file(&pool, id).await.unwrap());
        assert!(!delete_file(&pool, id).await.unwrap());
        assert_eq!(list_files(&pool).await.unwrap().len(), 1);
    }
}
