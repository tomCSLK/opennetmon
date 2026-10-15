//! Persistence of devices.

use sqlx::{FromRow, SqlitePool};

use crate::devices::{Device, DeviceInput, Polling};
use crate::error::{Error, Result};

const SELECT: &str = "SELECT d.id, d.name, d.host, d.port, d.group_id, d.profile_id, p.name AS profile_name, \
                      d.interval_secs, d.timeout_secs, d.retries \
                      FROM devices d JOIN credential_profiles p ON p.id = d.profile_id";

#[derive(FromRow)]
struct Row {
    id: i64,
    name: String,
    host: String,
    port: i64,
    group_id: Option<i64>,
    profile_id: i64,
    profile_name: String,
    interval_secs: i64,
    timeout_secs: i64,
    retries: i64,
}

fn narrow<T: TryFrom<i64>>(value: i64, what: &str) -> Result<T> {
    T::try_from(value).map_err(|_| Error::Internal(format!("{what} out of range in devices table")))
}

impl TryFrom<Row> for Device {
    type Error = Error;

    fn try_from(r: Row) -> Result<Self> {
        Ok(Self {
            id: r.id,
            name: r.name,
            host: r.host,
            group_id: r.group_id,
            profile_id: r.profile_id,
            profile_name: r.profile_name,
            polling: Polling {
                port: narrow(r.port, "port")?,
                interval_secs: narrow(r.interval_secs, "interval")?,
                timeout_secs: narrow(r.timeout_secs, "timeout")?,
                retries: narrow(r.retries, "retries")?,
            },
        })
    }
}

pub async fn list_devices(pool: &SqlitePool) -> Result<Vec<Device>> {
    sqlx::query_as::<_, Row>(&format!("{SELECT} ORDER BY d.name"))
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(Device::try_from)
        .collect()
}

pub async fn get_device(pool: &SqlitePool, id: i64) -> Result<Option<Device>> {
    sqlx::query_as::<_, Row>(&format!("{SELECT} WHERE d.id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .map(Device::try_from)
        .transpose()
}

fn map_write_error(e: sqlx::Error) -> Error {
    match e.as_database_error() {
        Some(d) if d.is_unique_violation() => {
            Error::Conflict("Ein Gerät mit diesem Namen existiert bereits.".to_owned())
        }
        Some(d) if d.is_foreign_key_violation() => Error::Invalid(
            "Die gewählte Gruppe oder das gewählte SNMP-Profil existiert nicht.".to_owned(),
        ),
        _ => e.into(),
    }
}

/// Creates (`id = None`) or updates a device and returns its id.
pub async fn save_device(pool: &SqlitePool, id: Option<i64>, input: DeviceInput) -> Result<i64> {
    let input = input
        .validated()
        .map_err(|m| Error::Invalid(m.to_owned()))?;
    if id.is_some() && get_device(pool, id.unwrap_or_default()).await?.is_none() {
        return Err(Error::NotFound);
    }
    let sql = match id {
        None => {
            "INSERT INTO devices (name, host, port, group_id, profile_id, interval_secs, timeout_secs, retries) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
        }
        Some(_) => {
            "UPDATE devices SET name = ?, host = ?, port = ?, group_id = ?, profile_id = ?, interval_secs = ?, \
             timeout_secs = ?, retries = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?"
        }
    };
    let query = sqlx::query(sql)
        .bind(&input.name)
        .bind(&input.host)
        .bind(i64::from(input.polling.port))
        .bind(input.group_id)
        .bind(input.profile_id)
        .bind(i64::from(input.polling.interval_secs))
        .bind(i64::from(input.polling.timeout_secs))
        .bind(i64::from(input.polling.retries));
    let query = match id {
        Some(id) => query.bind(id),
        None => query,
    };
    let done = query.execute(pool).await.map_err(map_write_error)?;
    Ok(id.unwrap_or_else(|| done.last_insert_rowid()))
}

/// Returns true if a device was deleted.
pub async fn delete_device(pool: &SqlitePool, id: i64) -> Result<bool> {
    let done = sqlx::query("DELETE FROM devices WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::{AuthProtocol, ProfileInput, ProfileKind, Secret, SecurityLevel};
    use crate::crypto::SecretBox;
    use crate::storage::{self, credentials, groups};

    async fn setup() -> (SqlitePool, i64, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pool = storage::connect(&dir.path().join("d.db")).await.unwrap();
        storage::migrate(&pool).await.unwrap();
        let input = ProfileInput {
            name: "Omada".to_owned(),
            kind: ProfileKind::V3 {
                username: "MCS6_SNMP".to_owned(),
                level: SecurityLevel::AuthNoPriv,
                auth_protocol: AuthProtocol::Md5,
                auth_password: Some(Secret::new("12345678")),
                priv_protocol: None,
                priv_password: None,
            },
        };
        let profile =
            credentials::save_profile(&pool, &SecretBox::from_key([3u8; 32]), None, &input)
                .await
                .unwrap();
        (pool, profile, dir)
    }

    fn input(name: &str, host: &str, group: Option<i64>, profile: i64) -> DeviceInput {
        DeviceInput {
            name: name.to_owned(),
            host: host.to_owned(),
            group_id: group,
            profile_id: profile,
            polling: Polling::default(),
        }
    }

    #[tokio::test]
    async fn create_get_list_update_delete() {
        let (pool, profile, _dir) = setup().await;
        let group = groups::create_group(&pool, "Keller", None).await.unwrap();
        let id = save_device(
            &pool,
            None,
            input(" Switch Keller ", "192.168.178.251", Some(group), profile),
        )
        .await
        .unwrap();

        let d = get_device(&pool, id).await.unwrap().unwrap();
        assert_eq!(
            (d.name.as_str(), d.host.as_str(), d.group_id),
            ("Switch Keller", "192.168.178.251", Some(group))
        );
        assert_eq!(
            (d.profile_name.as_str(), d.polling),
            ("Omada", Polling::default())
        );

        let mut changed = input("Switch Keller", "SW-Keller.LAN", None, profile);
        changed.polling = Polling {
            port: 1161,
            interval_secs: 30,
            timeout_secs: 3,
            retries: 2,
        };
        save_device(&pool, Some(id), changed).await.unwrap();
        let d = get_device(&pool, id).await.unwrap().unwrap();
        assert_eq!(
            (
                d.host.as_str(),
                d.group_id,
                d.polling.port,
                d.polling.interval_secs
            ),
            ("sw-keller.lan", None, 1161, 30)
        );
        assert_eq!(list_devices(&pool).await.unwrap().len(), 1);

        assert!(delete_device(&pool, id).await.unwrap());
        assert!(!delete_device(&pool, id).await.unwrap());
        assert!(get_device(&pool, id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn validation_uniqueness_and_missing_references() {
        let (pool, profile, _dir) = setup().await;
        save_device(&pool, None, input("A", "10.0.0.1", None, profile))
            .await
            .unwrap();
        assert!(matches!(
            save_device(&pool, None, input("a", "10.0.0.2", None, profile)).await,
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            save_device(&pool, None, input("B", "bad host", None, profile)).await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            save_device(&pool, None, input("C", "10.0.0.3", Some(99), profile)).await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            save_device(&pool, None, input("D", "10.0.0.4", None, 99)).await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            save_device(&pool, Some(99), input("E", "10.0.0.5", None, profile)).await,
            Err(Error::NotFound)
        ));
        assert_eq!(list_devices(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn used_profiles_and_groups_cannot_be_deleted() {
        let (pool, profile, _dir) = setup().await;
        let group = groups::create_group(&pool, "G", None).await.unwrap();
        let id = save_device(&pool, None, input("A", "10.0.0.1", Some(group), profile))
            .await
            .unwrap();

        // A profile that a device uses cannot be deleted (checked, and enforced by the foreign key).
        assert!(matches!(
            credentials::delete_profile(&pool, profile).await,
            Err(Error::Conflict(_))
        ));
        assert!(
            sqlx::query("DELETE FROM credential_profiles WHERE id = ?")
                .bind(profile)
                .execute(&pool)
                .await
                .is_err()
        );
        assert!(matches!(
            groups::delete_group(&pool, group).await,
            Err(Error::Conflict(_))
        ));

        delete_device(&pool, id).await.unwrap();
        groups::delete_group(&pool, group).await.unwrap();
        assert!(credentials::delete_profile(&pool, profile).await.unwrap());
    }
}
