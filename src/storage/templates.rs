//! Persistence of device templates and of the template each device was set up with.

use sqlx::{FromRow, SqlitePool};

use crate::error::{Error, Result};
use crate::i18n::Msg;
use crate::ordering::natural_cmp;
use crate::templates::Body;

/// Longest accepted `sysObjectID` text.
pub const MAX_OBJECT_ID_LEN: usize = 300;

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateRecord {
    pub id: i64,
    pub name: String,
    pub object_id: Option<String>,
    pub source: String,
    pub body: Body,
    pub created_at: String,
}

#[derive(FromRow)]
struct Row {
    id: i64,
    name: String,
    object_id: Option<String>,
    source: String,
    body: String,
    created_at: String,
}

impl TryFrom<Row> for TemplateRecord {
    type Error = Error;

    fn try_from(r: Row) -> Result<Self> {
        Ok(Self {
            body: Body::from_json(&r.body)?,
            id: r.id,
            name: r.name,
            object_id: r.object_id,
            source: r.source,
            created_at: r.created_at,
        })
    }
}

const SELECT: &str = "SELECT id, name, object_id, source, body, created_at FROM device_templates";

/// Empty means none; otherwise a dotted OID such as `1.3.6.1.4.1.11863.1.1.11`.
pub fn normalize_object_id(text: &str) -> Result<Option<String>> {
    let text = text.trim().trim_start_matches('.');
    if text.is_empty() {
        return Ok(None);
    }
    let valid = text.len() <= MAX_OBJECT_ID_LEN
        && text.split('.').count() >= 2
        && text
            .split('.')
            .all(|p| !p.is_empty() && p.parse::<u64>().is_ok());
    if !valid {
        return Err(Error::invalid(Msg::new("template-object-id-invalid")));
    }
    Ok(Some(text.to_owned()))
}

fn name_taken() -> Error {
    Error::conflict(Msg::new("template-name-taken"))
}

fn map_write_error(e: sqlx::Error) -> Error {
    match e.as_database_error() {
        Some(d) if d.is_unique_violation() => name_taken(),
        _ => e.into(),
    }
}

/// All templates, by name.
pub async fn list_templates(pool: &SqlitePool) -> Result<Vec<TemplateRecord>> {
    let mut list = sqlx::query_as::<_, Row>(SELECT)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(TemplateRecord::try_from)
        .collect::<Result<Vec<_>>>()?;
    list.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    Ok(list)
}

pub async fn get_template(pool: &SqlitePool, id: i64) -> Result<Option<TemplateRecord>> {
    sqlx::query_as::<_, Row>(&format!("{SELECT} WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .map(TemplateRecord::try_from)
        .transpose()
}

pub async fn create_template(
    pool: &SqlitePool,
    name: &str,
    object_id: Option<&str>,
    source: &str,
    body: &Body,
) -> Result<i64> {
    let name = crate::devices::validate_name(name).map_err(Error::invalid)?;
    let object_id = normalize_object_id(object_id.unwrap_or_default())?;
    body.validate().map_err(Error::invalid)?;
    let done = sqlx::query(
        "INSERT INTO device_templates (name, object_id, source, body) VALUES (?, ?, ?, ?)",
    )
    .bind(name)
    .bind(object_id)
    .bind(
        source
            .chars()
            .take(crate::devices::MAX_NAME_LEN)
            .collect::<String>(),
    )
    .bind(body.to_json()?)
    .execute(pool)
    .await
    .map_err(map_write_error)?;
    Ok(done.last_insert_rowid())
}

/// Changes name and device identification; the rules stay.
pub async fn update_template(
    pool: &SqlitePool,
    id: i64,
    name: &str,
    object_id: &str,
) -> Result<()> {
    let name = crate::devices::validate_name(name).map_err(Error::invalid)?;
    let object_id = normalize_object_id(object_id)?;
    let done = sqlx::query("UPDATE device_templates SET name = ?, object_id = ? WHERE id = ?")
        .bind(name)
        .bind(object_id)
        .bind(id)
        .execute(pool)
        .await
        .map_err(map_write_error)?;
    if done.rows_affected() == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

/// Deletes a template; devices set up with it keep their sensors.
pub async fn delete_template(pool: &SqlitePool, id: i64) -> Result<bool> {
    let done = sqlx::query("DELETE FROM device_templates WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// The template a device was last set up with.
pub async fn template_of_device(pool: &SqlitePool, device_id: i64) -> Result<Option<i64>> {
    Ok(
        sqlx::query_scalar::<_, Option<i64>>("SELECT template_id FROM devices WHERE id = ?")
            .bind(device_id)
            .fetch_optional(pool)
            .await?
            .flatten(),
    )
}

pub async fn set_template_of_device(
    pool: &SqlitePool,
    device_id: i64,
    template_id: Option<i64>,
) -> Result<()> {
    sqlx::query("UPDATE devices SET template_id = ? WHERE id = ?")
        .bind(template_id)
        .bind(device_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// How many devices were set up with each template.
pub async fn usage(pool: &SqlitePool) -> Result<Vec<(i64, i64)>> {
    Ok(sqlx::query_as::<_, (i64, i64)>(
        "SELECT template_id, COUNT(*) FROM devices WHERE template_id IS NOT NULL GROUP BY template_id",
    )
    .fetch_all(pool)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_ids_are_dotted_numbers_or_nothing() {
        assert_eq!(normalize_object_id("  ").unwrap(), None);
        assert_eq!(
            normalize_object_id(" .1.3.6.1.4.1.11863.1.1.11 ")
                .unwrap()
                .as_deref(),
            Some("1.3.6.1.4.1.11863.1.1.11")
        );
        for bad in ["abc", "1", "1..3", "1.3.x", &"1.".repeat(200)] {
            assert!(normalize_object_id(bad).is_err(), "{bad}");
        }
    }
}
