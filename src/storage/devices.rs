//! Persistence of devices.

use sqlx::{FromRow, SqlitePool};

use crate::devices::{Device, DeviceInput, DeviceKind, Polling};
use crate::error::{Error, Result};
use crate::i18n::Msg;
use crate::ordering::{natural_cmp, position_cmp};
use crate::storage::{OrderScope, write_positions};

const SELECT: &str = "SELECT d.id, d.name, d.host, d.port, d.group_id, d.profile_id, p.name AS profile_name, \
                      d.interval_secs, d.timeout_secs, d.retries, d.position, d.kind \
                      FROM devices d LEFT JOIN credential_profiles p ON p.id = d.profile_id";

#[derive(FromRow)]
struct Row {
    id: i64,
    name: String,
    host: String,
    port: i64,
    group_id: Option<i64>,
    profile_id: Option<i64>,
    profile_name: Option<String>,
    interval_secs: i64,
    timeout_secs: i64,
    retries: i64,
    position: Option<i64>,
    kind: String,
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
            kind: DeviceKind::parse(&r.kind).unwrap_or_default(),
        })
    }
}

/// All devices in display order: manual position, then natural name order (devices of one group
/// compare correctly).
pub async fn list_devices(pool: &SqlitePool) -> Result<Vec<Device>> {
    let mut rows = sqlx::query_as::<_, Row>(SELECT).fetch_all(pool).await?;
    rows.sort_by(|a, b| {
        position_cmp(a.position, b.position).then_with(|| natural_cmp(&a.name, &b.name))
    });
    rows.into_iter().map(Device::try_from).collect()
}

/// Stores the manual order of the devices in `group` (`None` = without group); empty = automatic.
pub async fn set_positions(pool: &SqlitePool, group: Option<i64>, ids: &[i64]) -> Result<()> {
    write_positions(pool, OrderScope::Devices(group), ids).await
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
        Some(d) if d.is_unique_violation() => Error::conflict(Msg::new("device-name-taken")),
        Some(d) if d.is_foreign_key_violation() => {
            Error::invalid(Msg::new("device-group-or-profile-missing"))
        }
        _ => e.into(),
    }
}

/// Creates (`id = None`) or updates a device and returns its id.
pub async fn save_device(pool: &SqlitePool, id: Option<i64>, input: DeviceInput) -> Result<i64> {
    let input = input.validated().map_err(Error::Invalid)?;
    if id.is_some() && get_device(pool, id.unwrap_or_default()).await?.is_none() {
        return Err(Error::NotFound);
    }
    let sql = match id {
        None => {
            "INSERT INTO devices (name, host, port, group_id, profile_id, interval_secs, timeout_secs, retries, kind) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
        }
        Some(_) => {
            // A device moved to another group loses its manual place there (the CASE sees the old row).
            "UPDATE devices SET position = CASE WHEN group_id IS ?4 THEN position ELSE NULL END, \
             name = ?1, host = ?2, port = ?3, group_id = ?4, profile_id = ?5, interval_secs = ?6, \
             timeout_secs = ?7, retries = ?8, kind = ?9, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?10"
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
        .bind(i64::from(input.polling.retries))
        .bind(input.kind.as_str());
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
            kind: Default::default(),
            name: name.to_owned(),
            host: host.to_owned(),
            group_id: group,
            profile_id: Some(profile),
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
            (d.profile_name.as_deref().unwrap_or_default(), d.polling),
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

    #[tokio::test]
    async fn devices_and_groups_keep_a_manual_order_until_they_move() {
        let (pool, profile, _dir) = setup().await;
        let haus = groups::create_group(&pool, "Haus", None).await.unwrap();
        let keller = groups::create_group(&pool, "Keller", None).await.unwrap();
        let mut ids = Vec::new();
        for name in ["Switch 10", "switch 2", "AP"] {
            ids.push(
                save_device(&pool, None, input(name, "192.0.2.1", Some(haus), profile))
                    .await
                    .unwrap(),
            );
        }
        let names = || async {
            list_devices(&pool)
                .await
                .unwrap()
                .into_iter()
                .filter(|d| d.group_id == Some(haus))
                .map(|d| d.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names().await, ["AP", "switch 2", "Switch 10"]);
        set_positions(&pool, Some(haus), &[ids[0], ids[2]])
            .await
            .unwrap();
        assert_eq!(names().await, ["Switch 10", "AP", "switch 2"]);

        // Saving without moving keeps the place, moving to another group drops it.
        save_device(
            &pool,
            Some(ids[0]),
            input("Switch 10", "192.0.2.9", Some(haus), profile),
        )
        .await
        .unwrap();
        assert_eq!(names().await[0], "Switch 10");
        save_device(
            &pool,
            Some(ids[0]),
            input("Switch 10", "192.0.2.9", Some(keller), profile),
        )
        .await
        .unwrap();
        save_device(
            &pool,
            Some(ids[0]),
            input("Switch 10", "192.0.2.9", Some(haus), profile),
        )
        .await
        .unwrap();
        assert_eq!(names().await, ["AP", "switch 2", "Switch 10"]);

        // Groups: same rules per parent.
        let top = || async {
            groups::list_groups(&pool)
                .await
                .unwrap()
                .into_iter()
                .filter(|g| g.parent_id.is_none())
                .map(|g| g.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(top().await, ["Haus", "Keller"]);
        groups::set_positions(&pool, None, &[keller, haus])
            .await
            .unwrap();
        assert_eq!(top().await, ["Keller", "Haus"]);
        groups::update_group(&pool, keller, "Keller", Some(haus))
            .await
            .unwrap();
        groups::update_group(&pool, keller, "Keller", None)
            .await
            .unwrap();
        groups::set_positions(&pool, None, &[haus]).await.unwrap();
        assert_eq!(top().await, ["Haus", "Keller"]);
        groups::set_positions(&pool, None, &[keller, haus])
            .await
            .unwrap();
        groups::update_group(&pool, keller, "Kellerraum", None)
            .await
            .unwrap();
        assert_eq!(
            top().await,
            ["Kellerraum", "Haus"],
            "renaming keeps the place"
        );
    }
}
