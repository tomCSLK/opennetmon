//! Persistence of sensors, channels and samples.

use sqlx::{FromRow, SqlitePool};

use crate::error::{Error, Result};
use crate::sensors::Unit;
use crate::sensors::snmp::OidMode;
use crate::sensors::spec::{SensorSpec, format_oid, parse_oid};

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelRecord {
    pub id: i64,
    pub key: String,
    pub unit: Unit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SensorRecord {
    pub id: i64,
    pub device_id: i64,
    pub name: String,
    pub spec: SensorSpec,
    pub enabled: bool,
    pub channels: Vec<ChannelRecord>,
}

/// One stored measurement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub channel_id: i64,
    pub ts_ms: i64,
    pub value: f64,
}

#[derive(FromRow)]
struct SensorRow {
    id: i64,
    device_id: i64,
    name: String,
    kind: String,
    if_index: Option<i64>,
    oid: Option<String>,
    oid_mode: Option<String>,
    scale: Option<f64>,
    enabled: i64,
}

#[derive(FromRow)]
struct ChannelRow {
    id: i64,
    sensor_id: i64,
    key: String,
    unit: String,
}

fn corrupt(what: &str) -> Error {
    Error::Internal(format!("invalid {what} in sensors table"))
}

fn spec_from_row(r: &SensorRow) -> Result<SensorSpec> {
    let if_index = || {
        r.if_index
            .and_then(|i| u32::try_from(i).ok())
            .ok_or_else(|| corrupt("if_index"))
    };
    Ok(match r.kind.as_str() {
        "ping" => SensorSpec::Ping,
        "uptime" => SensorSpec::Uptime,
        "if_traffic" => SensorSpec::InterfaceTraffic {
            if_index: if_index()?,
        },
        "if_status" => SensorSpec::InterfaceStatus {
            if_index: if_index()?,
        },
        "oid" => SensorSpec::Oid {
            oid: parse_oid(r.oid.as_deref().unwrap_or_default()).map_err(|_| corrupt("oid"))?,
            mode: match r.oid_mode.as_deref() {
                Some("gauge") => OidMode::Gauge,
                Some("counter") => OidMode::Counter,
                _ => return Err(corrupt("oid_mode")),
            },
            scale: r.scale.ok_or_else(|| corrupt("scale"))?,
        },
        _ => return Err(corrupt("kind")),
    })
}

const SENSOR_SELECT: &str =
    "SELECT id, device_id, name, kind, if_index, oid, oid_mode, scale, enabled FROM sensors";

async fn assemble(pool: &SqlitePool, rows: Vec<SensorRow>) -> Result<Vec<SensorRecord>> {
    let channels = sqlx::query_as::<_, ChannelRow>(
        "SELECT id, sensor_id, key, unit FROM channels ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            let channels = channels
                .iter()
                .filter(|c| c.sensor_id == r.id)
                .map(|c| {
                    Ok(ChannelRecord {
                        id: c.id,
                        key: c.key.clone(),
                        unit: Unit::parse(&c.unit).ok_or_else(|| corrupt("unit"))?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(SensorRecord {
                id: r.id,
                device_id: r.device_id,
                name: r.name.clone(),
                spec: spec_from_row(&r)?,
                enabled: r.enabled != 0,
                channels,
            })
        })
        .collect()
}

pub async fn list_sensors(pool: &SqlitePool, device_id: i64) -> Result<Vec<SensorRecord>> {
    let rows = sqlx::query_as::<_, SensorRow>(&format!(
        "{SENSOR_SELECT} WHERE device_id = ? ORDER BY name"
    ))
    .bind(device_id)
    .fetch_all(pool)
    .await?;
    assemble(pool, rows).await
}

/// All enabled sensors of all devices, as the poller needs them.
pub async fn list_enabled_sensors(pool: &SqlitePool) -> Result<Vec<SensorRecord>> {
    let rows = sqlx::query_as::<_, SensorRow>(&format!(
        "{SENSOR_SELECT} WHERE enabled = 1 ORDER BY device_id, id"
    ))
    .fetch_all(pool)
    .await?;
    assemble(pool, rows).await
}

pub async fn get_sensor(pool: &SqlitePool, id: i64) -> Result<Option<SensorRecord>> {
    let rows = sqlx::query_as::<_, SensorRow>(&format!("{SENSOR_SELECT} WHERE id = ?"))
        .bind(id)
        .fetch_all(pool)
        .await?;
    Ok(assemble(pool, rows).await?.into_iter().next())
}

/// Creates a sensor and its channels in one transaction and returns the new id.
pub async fn create_sensor(
    pool: &SqlitePool,
    device_id: i64,
    name: &str,
    spec: &SensorSpec,
) -> Result<i64> {
    let name = crate::devices::validate_name(name).map_err(|m| Error::Invalid(m.to_owned()))?;
    spec.validate().map_err(|m| Error::Invalid(m.to_owned()))?;
    let (if_index, oid, oid_mode, scale) = match spec {
        SensorSpec::InterfaceTraffic { if_index } | SensorSpec::InterfaceStatus { if_index } => {
            (Some(i64::from(*if_index)), None, None, None)
        }
        SensorSpec::Oid { oid, mode, scale } => (
            None,
            Some(format_oid(oid)),
            Some(match mode {
                OidMode::Gauge => "gauge",
                OidMode::Counter => "counter",
            }),
            Some(*scale),
        ),
        SensorSpec::Ping | SensorSpec::Uptime => (None, None, None, None),
    };

    let mut tx = pool.begin().await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO sensors (device_id, name, kind, if_index, oid, oid_mode, scale) \
         VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(device_id)
    .bind(name)
    .bind(spec.kind())
    .bind(if_index)
    .bind(oid)
    .bind(oid_mode)
    .bind(scale)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match e.as_database_error() {
        Some(d) if d.is_unique_violation() => Error::Conflict(
            "Ein Sensor mit diesem Namen existiert an diesem Gerät bereits.".to_owned(),
        ),
        Some(d) if d.is_foreign_key_violation() => {
            Error::Invalid("Das gewählte Gerät existiert nicht.".to_owned())
        }
        _ => e.into(),
    })?;
    for channel in spec.channels() {
        sqlx::query("INSERT INTO channels (sensor_id, key, unit) VALUES (?, ?, ?)")
            .bind(id)
            .bind(channel.key)
            .bind(channel.unit.as_str())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(id)
}

pub async fn set_enabled(pool: &SqlitePool, id: i64, enabled: bool) -> Result<()> {
    let done = sqlx::query("UPDATE sensors SET enabled = ? WHERE id = ?")
        .bind(i64::from(enabled))
        .bind(id)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

/// Deletes the sensor together with its channels and samples.
pub async fn delete_sensor(pool: &SqlitePool, id: i64) -> Result<()> {
    let done = sqlx::query("DELETE FROM sensors WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

/// Stores samples in one transaction. A sample at an already stored timestamp is ignored.
pub async fn insert_samples(pool: &SqlitePool, samples: &[Sample]) -> Result<()> {
    if samples.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for s in samples {
        sqlx::query("INSERT OR IGNORE INTO samples (channel_id, ts, value) VALUES (?, ?, ?)")
            .bind(s.channel_id)
            .bind(s.ts_ms)
            .bind(s.value)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Samples of one channel with `ts_ms >= since_ms`, oldest first.
pub async fn samples_since(
    pool: &SqlitePool,
    channel_id: i64,
    since_ms: i64,
) -> Result<Vec<Sample>> {
    let rows: Vec<(i64, f64)> = sqlx::query_as(
        "SELECT ts, value FROM samples WHERE channel_id = ? AND ts >= ? ORDER BY ts",
    )
    .bind(channel_id)
    .bind(since_ms)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(ts_ms, value)| Sample {
            channel_id,
            ts_ms,
            value,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{connect, migrate};

    /// A migrated database with one device (and the profile it needs).
    async fn setup() -> (tempfile::TempDir, SqlitePool, i64) {
        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("t.db")).await.unwrap();
        migrate(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO credential_profiles (name, version, community_enc) VALUES ('p', 'v2c', x'01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let device_id: i64 = sqlx::query_scalar(
            "INSERT INTO devices (name, host, profile_id) VALUES ('sw', '192.0.2.1', 1) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        (dir, pool, device_id)
    }

    fn oid_spec() -> SensorSpec {
        SensorSpec::Oid {
            oid: vec![1, 3, 6, 1, 4, 1, 99, 1, 0],
            mode: OidMode::Counter,
            scale: 0.5,
        }
    }

    #[tokio::test]
    async fn every_kind_round_trips_with_its_channels() {
        let (_dir, pool, device) = setup().await;
        let specs = [
            ("Ping", SensorSpec::Ping),
            ("Uptime", SensorSpec::Uptime),
            (
                "Traffic 1",
                SensorSpec::InterfaceTraffic { if_index: 49153 },
            ),
            ("Status 1", SensorSpec::InterfaceStatus { if_index: 3 }),
            ("Custom", oid_spec()),
        ];
        for (name, spec) in &specs {
            create_sensor(&pool, device, name, spec).await.unwrap();
        }
        let stored = list_sensors(&pool, device).await.unwrap();
        assert_eq!(stored.len(), 5);
        for (name, spec) in &specs {
            let record = stored.iter().find(|s| s.name == *name).unwrap();
            assert_eq!(&record.spec, spec);
            assert!(record.enabled);
            let keys: Vec<_> = record.channels.iter().map(|c| c.key.as_str()).collect();
            let expected: Vec<_> = spec.channels().iter().map(|c| c.key).collect();
            assert_eq!(keys, expected);
        }
    }

    #[tokio::test]
    async fn names_are_unique_per_device_and_case_insensitive() {
        let (_dir, pool, device) = setup().await;
        create_sensor(&pool, device, "Ping", &SensorSpec::Ping)
            .await
            .unwrap();
        let err = create_sensor(&pool, device, "PING", &SensorSpec::Uptime)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    }

    #[tokio::test]
    async fn invalid_input_and_unknown_device_are_rejected() {
        let (_dir, pool, device) = setup().await;
        for (id, name, spec) in [
            (device, "", SensorSpec::Ping),
            (device, "x", SensorSpec::InterfaceStatus { if_index: 0 }),
            (device + 1, "x", SensorSpec::Ping),
        ] {
            let err = create_sensor(&pool, id, name, &spec).await.unwrap_err();
            assert!(matches!(err, Error::Invalid(_)), "{err:?}");
        }
        assert!(list_sensors(&pool, device).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn disabled_sensors_are_not_listed_for_polling() {
        let (_dir, pool, device) = setup().await;
        let a = create_sensor(&pool, device, "A", &SensorSpec::Ping)
            .await
            .unwrap();
        create_sensor(&pool, device, "B", &SensorSpec::Uptime)
            .await
            .unwrap();
        set_enabled(&pool, a, false).await.unwrap();
        let enabled = list_enabled_sensors(&pool).await.unwrap();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].name, "B");
        assert!(matches!(
            set_enabled(&pool, 999, true).await,
            Err(Error::NotFound)
        ));
    }

    #[tokio::test]
    async fn samples_are_stored_ordered_and_duplicates_ignored() {
        let (_dir, pool, device) = setup().await;
        let id = create_sensor(
            &pool,
            device,
            "T",
            &SensorSpec::InterfaceTraffic { if_index: 1 },
        )
        .await
        .unwrap();
        let channel = get_sensor(&pool, id).await.unwrap().unwrap().channels[0].id;
        let sample = |ts_ms, value| Sample {
            channel_id: channel,
            ts_ms,
            value,
        };
        insert_samples(
            &pool,
            &[sample(2_000, 2.0), sample(1_000, 1.0), sample(3_000, 3.0)],
        )
        .await
        .unwrap();
        insert_samples(&pool, &[sample(2_000, 99.0)]).await.unwrap();
        insert_samples(&pool, &[]).await.unwrap();
        let got = samples_since(&pool, channel, 2_000).await.unwrap();
        assert_eq!(got, [sample(2_000, 2.0), sample(3_000, 3.0)]);
    }

    #[tokio::test]
    async fn deleting_a_sensor_or_device_removes_channels_and_samples() {
        let (_dir, pool, device) = setup().await;
        let id = create_sensor(&pool, device, "U", &SensorSpec::Uptime)
            .await
            .unwrap();
        let channel = get_sensor(&pool, id).await.unwrap().unwrap().channels[0].id;
        insert_samples(
            &pool,
            &[Sample {
                channel_id: channel,
                ts_ms: 1,
                value: 1.0,
            }],
        )
        .await
        .unwrap();
        delete_sensor(&pool, id).await.unwrap();
        assert!(get_sensor(&pool, id).await.unwrap().is_none());
        assert!(samples_since(&pool, channel, 0).await.unwrap().is_empty());
        assert!(matches!(
            delete_sensor(&pool, id).await,
            Err(Error::NotFound)
        ));

        create_sensor(&pool, device, "U2", &SensorSpec::Uptime)
            .await
            .unwrap();
        sqlx::query("DELETE FROM devices WHERE id = ?")
            .bind(device)
            .execute(&pool)
            .await
            .unwrap();
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channels")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 0);
    }
}
