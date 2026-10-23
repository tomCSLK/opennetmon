//! Persistence of sensors, channels and samples.

use sqlx::{FromRow, SqlitePool};

use crate::error::{Error, Result};
use crate::i18n::Msg;
use crate::ordering::{natural_cmp, position_cmp};
use crate::sensors::Unit;
use crate::sensors::local::LocalMetric;
use crate::sensors::snmp::OidMode;
use crate::sensors::spec::{SensorSpec, format_oid, parse_oid};
use crate::status::{Status, Thresholds, ValueState};
use crate::storage::{OrderScope, write_positions};

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelRecord {
    pub id: i64,
    pub key: String,
    pub unit: Unit,
    pub thresholds: Thresholds,
    /// Ratings of status values (empty = not rated).
    pub states: Vec<ValueState>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SensorRecord {
    pub id: i64,
    pub device_id: i64,
    pub name: String,
    pub spec: SensorSpec,
    pub enabled: bool,
    /// Own polling interval in seconds; `None` = the device's.
    pub interval_secs: Option<u32>,
    pub channels: Vec<ChannelRecord>,
}

impl SensorRecord {
    /// The interval the sensor is polled at: its own, otherwise `device_interval`.
    pub fn interval(&self, device_interval: u32) -> u32 {
        self.interval_secs.unwrap_or(device_interval)
    }
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
    unit_label: Option<String>,
    target: Option<String>,
    enabled: i64,
    position: Option<i64>,
    interval_secs: Option<i64>,
}

#[derive(FromRow)]
struct ChannelRow {
    id: i64,
    sensor_id: i64,
    key: String,
    unit: String,
    warn_above: Option<f64>,
    warn_below: Option<f64>,
    down_above: Option<f64>,
    down_below: Option<f64>,
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
            unit: r.unit_label.clone(),
        },
        "local" => SensorSpec::Local(
            r.target
                .as_deref()
                .and_then(LocalMetric::parse)
                .ok_or_else(|| corrupt("target"))?,
        ),
        _ => return Err(corrupt("kind")),
    })
}

const SENSOR_SELECT: &str = "SELECT id, device_id, name, kind, if_index, oid, oid_mode, scale, unit_label, target, enabled, position, interval_secs FROM sensors";

/// Automatic order of sensor kinds: reachability first, then ports, then everything else.
fn kind_rank(kind: &str) -> u8 {
    match kind {
        "ping" => 0,
        "uptime" => 1,
        "if_traffic" => 2,
        "if_status" => 3,
        _ => 4,
    }
}

/// Display order within a device: manual position, then kind, then natural name order.
fn sort_rows(rows: &mut [SensorRow]) {
    rows.sort_by(|a, b| {
        a.device_id
            .cmp(&b.device_id)
            .then_with(|| position_cmp(a.position, b.position))
            .then_with(|| kind_rank(&a.kind).cmp(&kind_rank(&b.kind)))
            .then_with(|| natural_cmp(&a.name, &b.name))
    });
}

async fn assemble(pool: &SqlitePool, rows: Vec<SensorRow>) -> Result<Vec<SensorRecord>> {
    let channels = sqlx::query_as::<_, ChannelRow>(
        "SELECT id, sensor_id, key, unit, warn_above, warn_below, down_above, down_below \
         FROM channels ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    let mut states: std::collections::HashMap<i64, Vec<ValueState>> =
        std::collections::HashMap::new();
    for (channel_id, value, label, status) in sqlx::query_as::<_, (i64, i64, String, String)>(
        "SELECT channel_id, value, label, status FROM value_states ORDER BY channel_id, value",
    )
    .fetch_all(pool)
    .await?
    {
        states.entry(channel_id).or_default().push(ValueState {
            value,
            label,
            status: Status::parse(&status).ok_or_else(|| corrupt("value state"))?,
        });
    }
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
                        thresholds: Thresholds {
                            warn_above: c.warn_above,
                            warn_below: c.warn_below,
                            down_above: c.down_above,
                            down_below: c.down_below,
                        },
                        states: states.get(&c.id).cloned().unwrap_or_default(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(SensorRecord {
                id: r.id,
                device_id: r.device_id,
                name: r.name.clone(),
                spec: spec_from_row(&r)?,
                enabled: r.enabled != 0,
                interval_secs: r.interval_secs.and_then(|s| u32::try_from(s).ok()),
                channels,
            })
        })
        .collect()
}

/// The sensors of a device in display order.
pub async fn list_sensors(pool: &SqlitePool, device_id: i64) -> Result<Vec<SensorRecord>> {
    let mut rows = sqlx::query_as::<_, SensorRow>(&format!("{SENSOR_SELECT} WHERE device_id = ?"))
        .bind(device_id)
        .fetch_all(pool)
        .await?;
    sort_rows(&mut rows);
    assemble(pool, rows).await
}

/// Stores the manual order of a device's sensors; empty = automatic.
pub async fn set_positions(pool: &SqlitePool, device_id: i64, ids: &[i64]) -> Result<()> {
    write_positions(pool, OrderScope::Sensors(device_id), ids).await
}

/// Gives a sensor a new name (unique per device); history, thresholds and status stay.
pub async fn rename_sensor(pool: &SqlitePool, id: i64, name: &str) -> Result<()> {
    let name = crate::devices::validate_name(name).map_err(Error::invalid)?;
    let done = sqlx::query("UPDATE sensors SET name = ? WHERE id = ?")
        .bind(name)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| match e.as_database_error() {
            Some(d) if d.is_unique_violation() => name_taken(),
            _ => e.into(),
        })?;
    if done.rows_affected() == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

/// Sets a sensor's own polling interval (`None`: the device's), checked against the device's
/// timeout and retries like the device's own interval.
pub async fn set_interval(
    pool: &SqlitePool,
    id: i64,
    interval_secs: Option<u32>,
    polling: &crate::devices::Polling,
) -> Result<()> {
    if let Some(secs) = interval_secs {
        crate::devices::Polling {
            interval_secs: secs,
            ..*polling
        }
        .validate()
        .map_err(Error::invalid)?;
    }
    let done = sqlx::query("UPDATE sensors SET interval_secs = ? WHERE id = ?")
        .bind(interval_secs)
        .bind(id)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

fn name_taken() -> Error {
    Error::conflict(Msg::new("sensor-name-taken"))
}

/// All sensors of all devices (enabled or not), for overviews; by device, then display order.
pub async fn list_all_sensors(pool: &SqlitePool) -> Result<Vec<SensorRecord>> {
    let mut rows = sqlx::query_as::<_, SensorRow>(SENSOR_SELECT)
        .fetch_all(pool)
        .await?;
    sort_rows(&mut rows);
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
    create_sensor_with_state(pool, device_id, name, spec, true).await
}

/// Like [`create_sensor`], but the sensor may start inactive (not polled until activated).
pub async fn create_sensor_with_state(
    pool: &SqlitePool,
    device_id: i64,
    name: &str,
    spec: &SensorSpec,
    enabled: bool,
) -> Result<i64> {
    let name = crate::devices::validate_name(name).map_err(Error::invalid)?;
    spec.validate().map_err(Error::invalid)?;
    let (if_index, oid, oid_mode, scale, unit_label) = match spec {
        SensorSpec::InterfaceTraffic { if_index } | SensorSpec::InterfaceStatus { if_index } => {
            (Some(i64::from(*if_index)), None, None, None, None)
        }
        SensorSpec::Oid {
            oid,
            mode,
            scale,
            unit,
        } => (
            None,
            Some(format_oid(oid)),
            Some(match mode {
                OidMode::Gauge => "gauge",
                OidMode::Counter => "counter",
            }),
            Some(*scale),
            unit.as_deref().map(str::trim),
        ),
        SensorSpec::Ping | SensorSpec::Uptime | SensorSpec::Local(_) => {
            (None, None, None, None, None)
        }
    };
    let target = match spec {
        SensorSpec::Local(metric) => Some(metric.target()),
        _ => None,
    };

    let mut tx = pool.begin().await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO sensors (device_id, name, kind, if_index, oid, oid_mode, scale, unit_label, target, enabled) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(device_id)
    .bind(name)
    .bind(spec.kind())
    .bind(if_index)
    .bind(oid)
    .bind(oid_mode)
    .bind(scale)
    .bind(unit_label)
    .bind(target)
    .bind(i64::from(enabled))
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match e.as_database_error() {
        Some(d) if d.is_unique_violation() => name_taken(),
        Some(d) if d.is_foreign_key_violation() => {
            Error::invalid(Msg::new("sensor-device-missing"))
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

/// Replaces the thresholds of channels, all or none. Every channel must belong to `sensor_id`.
pub async fn set_thresholds(
    pool: &SqlitePool,
    sensor_id: i64,
    limits: &[(i64, Thresholds)],
) -> Result<()> {
    for (_, t) in limits {
        t.validate().map_err(Error::invalid)?;
    }
    let mut tx = pool.begin().await?;
    for (channel_id, t) in limits {
        let done = sqlx::query(
            "UPDATE channels SET warn_above = ?, warn_below = ?, down_above = ?, down_below = ? \
             WHERE id = ? AND sensor_id = ?",
        )
        .bind(t.warn_above)
        .bind(t.warn_below)
        .bind(t.down_above)
        .bind(t.down_below)
        .bind(channel_id)
        .bind(sensor_id)
        .execute(&mut *tx)
        .await?;
        if done.rows_affected() == 0 {
            return Err(Error::NotFound);
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Longest accepted name of a status value.
pub const MAX_STATE_LABEL_CHARS: usize = 64;

/// Replaces the ratings of status values of one channel of `sensor_id`; an empty list removes them.
pub async fn set_value_states(
    pool: &SqlitePool,
    sensor_id: i64,
    channel_id: i64,
    states: &[ValueState],
) -> Result<()> {
    for s in states {
        if !matches!(s.status, Status::Up | Status::Warning | Status::Down) {
            return Err(Error::invalid(Msg::new("states-rating-invalid")));
        }
        if s.label.chars().count() > MAX_STATE_LABEL_CHARS || s.label.chars().any(char::is_control)
        {
            return Err(Error::invalid(Msg::new("states-label-invalid")));
        }
    }
    let mut tx = pool.begin().await?;
    let owned: Option<i64> =
        sqlx::query_scalar("SELECT id FROM channels WHERE id = ? AND sensor_id = ?")
            .bind(channel_id)
            .bind(sensor_id)
            .fetch_optional(&mut *tx)
            .await?;
    if owned.is_none() {
        return Err(Error::NotFound);
    }
    sqlx::query("DELETE FROM value_states WHERE channel_id = ?")
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;
    for s in states {
        sqlx::query(
            "INSERT INTO value_states (channel_id, value, label, status) VALUES (?, ?, ?, ?) \
             ON CONFLICT (channel_id, value) DO UPDATE SET label = excluded.label, status = excluded.status",
        )
        .bind(channel_id)
        .bind(s.value)
        .bind(&s.label)
        .bind(s.status.as_str())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
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

/// Deletes several sensors of one device (with channels and samples); other devices' ids are ignored.
pub async fn delete_many(pool: &SqlitePool, device_id: i64, ids: &[i64]) -> Result<u64> {
    let mut tx = pool.begin().await?;
    let mut deleted = 0;
    for id in ids {
        deleted += sqlx::query("DELETE FROM sensors WHERE id = ? AND device_id = ?")
            .bind(id)
            .bind(device_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    }
    tx.commit().await?;
    Ok(deleted)
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
    use std::collections::HashMap;

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
            unit: Some("Pakete".to_owned()),
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
            ("CPU", SensorSpec::Local(LocalMetric::Cpu)),
            (
                "Daten",
                SensorSpec::Local(LocalMetric::Disk {
                    mount: "/srv/daten".to_owned(),
                }),
            ),
        ];
        for (name, spec) in &specs {
            create_sensor(&pool, device, name, spec).await.unwrap();
        }
        let stored = list_sensors(&pool, device).await.unwrap();
        assert_eq!(stored.len(), 7);
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
    async fn thresholds_are_stored_validated_and_scoped_to_the_sensor() {
        let (_dir, pool, device) = setup().await;
        let a = create_sensor(&pool, device, "A", &SensorSpec::Ping)
            .await
            .unwrap();
        let b = create_sensor(&pool, device, "B", &SensorSpec::Ping)
            .await
            .unwrap();
        let a_channel = get_sensor(&pool, a).await.unwrap().unwrap().channels[0].id;
        let b_channel = get_sensor(&pool, b).await.unwrap().unwrap().channels[0].id;
        assert!(
            get_sensor(&pool, a).await.unwrap().unwrap().channels[0]
                .thresholds
                .is_empty()
        );

        let t = Thresholds {
            warn_above: Some(50.0),
            down_above: Some(200.0),
            ..Thresholds::default()
        };
        set_thresholds(&pool, a, &[(a_channel, t)]).await.unwrap();
        assert_eq!(
            get_sensor(&pool, a).await.unwrap().unwrap().channels[0].thresholds,
            t
        );
        assert!(
            get_sensor(&pool, b).await.unwrap().unwrap().channels[0]
                .thresholds
                .is_empty()
        );

        // Foreign channel: nothing changes. Invalid limits: nothing changes.
        let other = Thresholds {
            warn_above: Some(1.0),
            ..Thresholds::default()
        };
        assert!(matches!(
            set_thresholds(&pool, a, &[(a_channel, other), (b_channel, other)]).await,
            Err(Error::NotFound)
        ));
        assert_eq!(
            get_sensor(&pool, a).await.unwrap().unwrap().channels[0].thresholds,
            t
        );
        let bad = Thresholds {
            warn_above: Some(9.0),
            down_above: Some(1.0),
            ..Thresholds::default()
        };
        assert!(matches!(
            set_thresholds(&pool, a, &[(a_channel, bad)]).await,
            Err(Error::Invalid(_))
        ));
        set_thresholds(&pool, a, &[(a_channel, Thresholds::default())])
            .await
            .unwrap();
        assert!(
            get_sensor(&pool, a).await.unwrap().unwrap().channels[0]
                .thresholds
                .is_empty()
        );
    }

    #[tokio::test]
    async fn sensors_can_start_inactive_and_be_deleted_in_bulk_per_device() {
        let (_dir, pool, device) = setup().await;
        let other: i64 = sqlx::query_scalar(
            "INSERT INTO devices (name, host, profile_id) VALUES ('ap', '192.0.2.2', 1) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let a = create_sensor_with_state(&pool, device, "A", &SensorSpec::Ping, false)
            .await
            .unwrap();
        let b = create_sensor_with_state(&pool, device, "B", &SensorSpec::Uptime, false)
            .await
            .unwrap();
        let foreign = create_sensor_with_state(&pool, other, "C", &SensorSpec::Ping, false)
            .await
            .unwrap();
        assert!(
            list_enabled_sensors(&pool).await.unwrap().is_empty(),
            "inactive sensors are not polled"
        );

        set_enabled(&pool, b, true).await.unwrap();
        assert_eq!(list_enabled_sensors(&pool).await.unwrap().len(), 1);

        assert_eq!(delete_many(&pool, device, &[a, foreign]).await.unwrap(), 1);
        assert!(get_sensor(&pool, a).await.unwrap().is_none());
        assert!(get_sensor(&pool, foreign).await.unwrap().is_some());
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

    #[tokio::test]
    async fn automatic_order_is_by_kind_then_natural_name_and_manual_order_wins() {
        let (_dir, pool, device) = setup().await;
        let mut ids = HashMap::new();
        for (name, spec) in [
            (
                "Traffic Port 10",
                SensorSpec::InterfaceTraffic { if_index: 10 },
            ),
            ("CPU", oid_spec()),
            ("Status Port 2", SensorSpec::InterfaceStatus { if_index: 2 }),
            (
                "traffic Port 2",
                SensorSpec::InterfaceTraffic { if_index: 2 },
            ),
            ("Uptime", SensorSpec::Uptime),
            ("Zz Ping", SensorSpec::Ping),
        ] {
            ids.insert(
                name,
                create_sensor(&pool, device, name, &spec).await.unwrap(),
            );
        }
        let names = || async {
            list_sensors(&pool, device)
                .await
                .unwrap()
                .into_iter()
                .map(|s| s.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names().await,
            [
                "Zz Ping",
                "Uptime",
                "traffic Port 2",
                "Traffic Port 10",
                "Status Port 2",
                "CPU"
            ]
        );

        // Manually placed sensors come first; the rest keeps the automatic order behind them.
        set_positions(&pool, device, &[ids["CPU"], ids["Traffic Port 10"], 9999])
            .await
            .unwrap();
        assert_eq!(
            names().await,
            [
                "CPU",
                "Traffic Port 10",
                "Zz Ping",
                "Uptime",
                "traffic Port 2",
                "Status Port 2"
            ]
        );
        let all: Vec<String> = list_all_sensors(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(all, names().await);

        set_positions(&pool, device, &[]).await.unwrap();
        assert_eq!(names().await[0], "Zz Ping", "reset to automatic");
    }

    #[tokio::test]
    async fn renaming_keeps_the_sensor_and_checks_the_name() {
        let (_dir, pool, device) = setup().await;
        let a = create_sensor(&pool, device, "A", &SensorSpec::Ping)
            .await
            .unwrap();
        create_sensor(&pool, device, "B", &SensorSpec::Uptime)
            .await
            .unwrap();
        rename_sensor(&pool, a, "  Erreichbarkeit  ").await.unwrap();
        let sensor = get_sensor(&pool, a).await.unwrap().unwrap();
        assert_eq!(
            (sensor.name.as_str(), sensor.channels.len()),
            ("Erreichbarkeit", 1)
        );
        assert!(matches!(
            rename_sensor(&pool, a, "B").await,
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            rename_sensor(&pool, a, "").await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            rename_sensor(&pool, a, &"x".repeat(65)).await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            rename_sensor(&pool, 999, "C").await,
            Err(Error::NotFound)
        ));
    }

    #[tokio::test]
    async fn value_states_are_stored_per_channel_and_scoped_to_the_sensor() {
        let (_dir, pool, device) = setup().await;
        let a = create_sensor(&pool, device, "A", &oid_spec())
            .await
            .unwrap();
        let b = create_sensor(&pool, device, "B", &SensorSpec::Uptime)
            .await
            .unwrap();
        let channel = get_sensor(&pool, a).await.unwrap().unwrap().channels[0].id;
        let state = |value, label: &str, status| ValueState {
            value,
            label: label.to_owned(),
            status,
        };
        let states = [state(1, "ok", Status::Up), state(3, "failed", Status::Down)];
        set_value_states(&pool, a, channel, &states).await.unwrap();
        assert_eq!(
            get_sensor(&pool, a).await.unwrap().unwrap().channels[0].states,
            states
        );
        assert!(matches!(
            set_value_states(&pool, b, channel, &states).await,
            Err(Error::NotFound)
        ));
        assert!(matches!(
            set_value_states(&pool, a, channel, &[state(1, "x", Status::Paused)]).await,
            Err(Error::Invalid(_))
        ));
        set_value_states(&pool, a, channel, &[]).await.unwrap();
        assert!(
            get_sensor(&pool, a).await.unwrap().unwrap().channels[0]
                .states
                .is_empty()
        );
        set_value_states(&pool, a, channel, &states).await.unwrap();
        delete_sensor(&pool, a).await.unwrap();
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM value_states")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 0, "ratings go with the sensor");
    }
}
