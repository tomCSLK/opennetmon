//! Persistence of maintenance windows (see [`crate::schedule`]): `schedules`, their
//! `schedule_windows`, the `schedule_assignments` of groups, devices and sensors and the log
//! `schedule_log`.

use std::collections::{HashMap, HashSet};

use sqlx::{SqliteConnection, SqlitePool};

use crate::error::{Error, Result};
use crate::i18n::Msg;
use crate::pause::{Pauses, ScheduleInfo, Target};
use crate::schedule::{self, Assignment, Window};
use crate::storage::pauses::LOG_KEPT;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowRecord {
    pub id: i64,
    pub window: Window,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    pub id: i64,
    pub name: String,
    /// Weekly windows first (by start time), then single ones (by start).
    pub windows: Vec<WindowRecord>,
    /// How many groups, devices and sensors it is assigned to.
    pub used: i64,
}

impl Schedule {
    pub fn plain_windows(&self) -> Vec<Window> {
        self.windows.iter().map(|w| w.window).collect()
    }
}

type WindowRow = (
    i64,
    i64,
    String,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

fn window(row: &WindowRow) -> Option<Window> {
    let (_, _, kind, weekdays, start_minute, duration, start_ms, end_ms) = row;
    match kind.as_str() {
        "weekly" => Some(Window::Weekly {
            weekdays: u8::try_from((*weekdays)?).ok()?,
            start_minute: u16::try_from((*start_minute)?).ok()?,
            duration_minutes: u32::try_from((*duration)?).ok()?,
        }),
        "once" => Some(Window::Once {
            start_ms: (*start_ms)?,
            end_ms: (*end_ms)?,
        }),
        _ => None,
    }
}

fn sort_key(w: &Window) -> (u8, i64) {
    match *w {
        Window::Weekly { start_minute, .. } => (0, i64::from(start_minute)),
        Window::Once { start_ms, .. } => (1, start_ms),
    }
}

async fn windows_by_schedule(
    conn: &mut SqliteConnection,
) -> Result<HashMap<i64, Vec<WindowRecord>>> {
    let rows: Vec<WindowRow> = sqlx::query_as(
        "SELECT id, schedule_id, kind, weekdays, start_minute, duration_minutes, start_ms, end_ms \
         FROM schedule_windows ORDER BY id",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut by: HashMap<i64, Vec<WindowRecord>> = HashMap::new();
    for row in &rows {
        if let Some(w) = window(row) {
            by.entry(row.1).or_default().push(WindowRecord {
                id: row.0,
                window: w,
            });
        }
    }
    for list in by.values_mut() {
        list.sort_by_key(|w| sort_key(&w.window));
    }
    Ok(by)
}

async fn list_on(conn: &mut SqliteConnection) -> Result<Vec<Schedule>> {
    let rows: Vec<(i64, String, i64)> = sqlx::query_as(
        "SELECT s.id, s.name, (SELECT COUNT(*) FROM schedule_assignments a WHERE a.schedule_id = s.id) \
         FROM schedules s",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut windows = windows_by_schedule(conn).await?;
    let mut list: Vec<Schedule> = rows
        .into_iter()
        .map(|(id, name, used)| Schedule {
            id,
            name,
            windows: windows.remove(&id).unwrap_or_default(),
            used,
        })
        .collect();
    list.sort_by(|a, b| crate::ordering::natural_cmp(&a.name, &b.name));
    Ok(list)
}

/// All schedules with their windows, by name.
pub async fn list(pool: &SqlitePool) -> Result<Vec<Schedule>> {
    list_on(&mut *pool.acquire().await?).await
}

pub async fn get(pool: &SqlitePool, id: i64) -> Result<Option<Schedule>> {
    Ok(list(pool).await?.into_iter().find(|s| s.id == id))
}

fn map_name_error(e: sqlx::Error) -> Error {
    match e.as_database_error() {
        Some(d) if d.is_unique_violation() => Error::conflict(Msg::new("schedule-name-taken")),
        _ => e.into(),
    }
}

/// Creates a schedule without windows; `name` must be [`schedule::validate_name`]d.
pub async fn create(pool: &SqlitePool, name: &str) -> Result<i64> {
    let id = sqlx::query_scalar("INSERT INTO schedules (name) VALUES (?) RETURNING id")
        .bind(name)
        .fetch_one(pool)
        .await
        .map_err(map_name_error)?;
    Ok(id)
}

pub async fn rename(pool: &SqlitePool, id: i64, name: &str) -> Result<()> {
    let done = sqlx::query("UPDATE schedules SET name = ? WHERE id = ?")
        .bind(name)
        .bind(id)
        .execute(pool)
        .await
        .map_err(map_name_error)?;
    if done.rows_affected() == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

/// Deletes a schedule; the objects it was assigned to inherit again. `false` if it did not exist.
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<bool> {
    let done = sqlx::query("DELETE FROM schedules WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Adds a window; errors with `NotFound` if the schedule is missing.
pub async fn add_window(pool: &SqlitePool, schedule_id: i64, w: Window) -> Result<i64> {
    let (kind, weekdays, start_minute, duration, start_ms, end_ms) = match w {
        Window::Weekly {
            weekdays,
            start_minute,
            duration_minutes,
        } => (
            "weekly",
            Some(i64::from(weekdays)),
            Some(i64::from(start_minute)),
            Some(i64::from(duration_minutes)),
            None,
            None,
        ),
        Window::Once { start_ms, end_ms } => {
            ("once", None, None, None, Some(start_ms), Some(end_ms))
        }
    };
    let id = sqlx::query_scalar(
        "INSERT INTO schedule_windows (schedule_id, kind, weekdays, start_minute, duration_minutes, start_ms, end_ms) \
         SELECT id, ?, ?, ?, ?, ?, ? FROM schedules WHERE id = ? RETURNING id",
    )
    .bind(kind)
    .bind(weekdays)
    .bind(start_minute)
    .bind(duration)
    .bind(start_ms)
    .bind(end_ms)
    .bind(schedule_id)
    .fetch_optional(pool)
    .await?;
    id.ok_or(Error::NotFound)
}

/// Deletes a window of a schedule; `false` if it did not exist.
pub async fn delete_window(pool: &SqlitePool, schedule_id: i64, window_id: i64) -> Result<bool> {
    let done = sqlx::query("DELETE FROM schedule_windows WHERE id = ? AND schedule_id = ?")
        .bind(window_id)
        .bind(schedule_id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

fn column(target: Target) -> &'static str {
    match target {
        Target::Group(_) => "group_id",
        Target::Device(_) => "device_id",
        Target::Sensor(_) => "sensor_id",
    }
}

fn table(target: Target) -> &'static str {
    match target {
        Target::Group(_) => "groups",
        Target::Device(_) => "devices",
        Target::Sensor(_) => "sensors",
    }
}

/// Stores which schedule applies to an object. Errors with `NotFound` if the object is missing and
/// with `Invalid` if the schedule is.
pub async fn assign(pool: &SqlitePool, target: Target, assignment: Assignment) -> Result<()> {
    let mut tx = pool.begin().await?;
    let exists: Option<i64> =
        sqlx::query_scalar(&format!("SELECT id FROM {} WHERE id = ?", table(target)))
            .bind(target.id())
            .fetch_optional(&mut *tx)
            .await?;
    if exists.is_none() {
        return Err(Error::NotFound);
    }
    if let Some(id) = assignment.schedule_id() {
        let found: Option<i64> = sqlx::query_scalar("SELECT id FROM schedules WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        if found.is_none() {
            return Err(Error::invalid(Msg::new("schedule-required")));
        }
    }
    let col = column(target);
    sqlx::query(&format!("DELETE FROM schedule_assignments WHERE {col} = ?"))
        .bind(target.id())
        .execute(&mut *tx)
        .await?;
    if assignment != Assignment::Parent {
        sqlx::query(&format!(
            "INSERT INTO schedule_assignments ({col}, schedule_id) VALUES (?, ?)"
        ))
        .bind(target.id())
        .bind(assignment.schedule_id())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Adds the schedules, with the end of any window open at `now_ms`, and their assignments to `pauses`.
pub(crate) async fn load_into(
    conn: &mut SqliteConnection,
    pauses: &mut Pauses,
    now_ms: i64,
) -> Result<()> {
    let tz = schedule::time_zone();
    for s in list_on(conn).await? {
        let open_until = schedule::active_until(&s.plain_windows(), now_ms, tz);
        pauses.schedules.insert(
            s.id,
            ScheduleInfo {
                name: s.name,
                open_until,
            },
        );
    }
    type AssignmentRow = (Option<i64>, Option<i64>, Option<i64>, Option<i64>);
    let rows: Vec<AssignmentRow> = sqlx::query_as(
        "SELECT group_id, device_id, sensor_id, schedule_id FROM schedule_assignments",
    )
    .fetch_all(&mut *conn)
    .await?;
    for (group, device, sensor, schedule) in rows {
        let assignment = schedule.map_or(Assignment::None, Assignment::Schedule);
        match (group, device, sensor) {
            (Some(id), _, _) => pauses.group_schedules.insert(id, assignment),
            (_, Some(id), _) => pauses.device_schedules.insert(id, assignment),
            (_, _, Some(id)) => pauses.sensor_schedules.insert(id, assignment),
            _ => None,
        };
    }
    Ok(())
}

/// Logs when a schedule starts and stops pausing sensors (one entry per window, not per sensor).
pub(crate) async fn log_windows(
    conn: &mut SqliteConnection,
    pauses: &Pauses,
    now_ms: i64,
) -> Result<()> {
    let roots = pauses.schedule_roots();
    let active: HashSet<i64> = sqlx::query_scalar::<_, i64>(
        "SELECT schedule_id FROM schedule_log l \
         WHERE id = (SELECT MAX(id) FROM schedule_log WHERE schedule_id = l.schedule_id) \
         AND action = 'started'",
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();
    let mut started: Vec<_> = roots
        .iter()
        .filter(|(id, _)| !active.contains(id))
        .collect();
    started.sort_by_key(|(id, _)| **id);
    let mut ended: Vec<_> = active.iter().filter(|id| !roots.contains_key(id)).collect();
    ended.sort_unstable();
    let count = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
    for (id, affected) in started {
        write_log(
            conn,
            now_ms,
            *id,
            "started",
            count(affected.sensors),
            count(affected.devices.len()),
        )
        .await?;
    }
    for id in ended {
        write_log(conn, now_ms, *id, "ended", 0, 0).await?;
    }
    Ok(())
}

async fn write_log(
    conn: &mut SqliteConnection,
    ts_ms: i64,
    schedule_id: i64,
    action: &str,
    sensors: i64,
    devices: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO schedule_log (ts_ms, schedule_id, action, sensors, devices) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(ts_ms)
    .bind(schedule_id)
    .bind(action)
    .bind(sensors)
    .bind(devices)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "DELETE FROM schedule_log WHERE id <= (SELECT id FROM schedule_log ORDER BY id DESC LIMIT 1 OFFSET ?)",
    )
    .bind(LOG_KEPT)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// One entry of the schedule log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub ts_ms: i64,
    /// `true` when a window started pausing, `false` when it ended.
    pub started: bool,
    pub schedule_id: i64,
    pub schedule: String,
    pub sensors: i64,
    pub devices: i64,
    /// For a start: when the next entry of the same schedule (its end) was written.
    pub end_ms: Option<i64>,
}

/// The schedule log, newest first.
pub async fn log_entries(pool: &SqlitePool) -> Result<Vec<LogEntry>> {
    type Row = (i64, String, i64, String, i64, i64, Option<i64>);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT l.ts_ms, l.action, l.schedule_id, s.name, l.sensors, l.devices, \
         LEAD(l.ts_ms) OVER (PARTITION BY l.schedule_id ORDER BY l.ts_ms, l.id) \
         FROM schedule_log l JOIN schedules s ON s.id = l.schedule_id \
         ORDER BY l.ts_ms DESC, l.id DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(ts_ms, action, schedule_id, schedule, sensors, devices, end_ms)| LogEntry {
                ts_ms,
                started: action == "started",
                schedule_id,
                schedule,
                sensors,
                devices,
                end_ms,
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{connect, migrate};

    async fn db() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let pool = connect(&dir.path().join("a.db")).await.unwrap();
        migrate(&pool).await.unwrap();
        (dir, pool)
    }

    #[tokio::test]
    async fn schedules_keep_their_windows_and_assignments() {
        let (_dir, pool) = db().await;
        let id = create(&pool, "Nachts").await.unwrap();
        assert!(matches!(
            create(&pool, "nachts").await,
            Err(Error::Conflict(_))
        ));
        let once = Window::Once {
            start_ms: 5_000,
            end_ms: 9_000,
        };
        let daily = Window::Weekly {
            weekdays: schedule::EVERY_DAY,
            start_minute: 180,
            duration_minutes: 15,
        };
        let w1 = add_window(&pool, id, once).await.unwrap();
        add_window(&pool, id, daily).await.unwrap();
        assert!(matches!(
            add_window(&pool, 99, daily).await,
            Err(Error::NotFound)
        ));
        let s = get(&pool, id).await.unwrap().unwrap();
        assert_eq!(s.plain_windows(), [daily, once], "weekly first");

        let group = crate::storage::groups::create_group(&pool, "Netzwerk", None)
            .await
            .unwrap();
        assign(&pool, Target::Group(group), Assignment::Schedule(id))
            .await
            .unwrap();
        assert!(matches!(
            assign(&pool, Target::Group(group), Assignment::Schedule(99)).await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            assign(&pool, Target::Device(99), Assignment::None).await,
            Err(Error::NotFound)
        ));
        assert_eq!(get(&pool, id).await.unwrap().unwrap().used, 1);
        let pauses = crate::storage::pauses::load_all(&pool, 6_000)
            .await
            .unwrap();
        assert_eq!(
            pauses.schedule_setting(Target::Group(group)),
            Assignment::Schedule(id)
        );
        assert_eq!(pauses.schedule_info(id).unwrap().open_until, Some(9_000));
        assert_eq!(pauses.group(group).and_then(|w| w.schedule()), Some(id));

        assign(&pool, Target::Group(group), Assignment::None)
            .await
            .unwrap();
        let pauses = crate::storage::pauses::load_all(&pool, 6_000)
            .await
            .unwrap();
        assert_eq!(
            pauses.schedule_setting(Target::Group(group)),
            Assignment::None
        );
        assign(&pool, Target::Group(group), Assignment::Schedule(id))
            .await
            .unwrap();

        assert!(delete_window(&pool, id, w1).await.unwrap());
        assert!(!delete_window(&pool, id, w1).await.unwrap());
        rename(&pool, id, "Nachtfenster").await.unwrap();
        assert!(delete(&pool, id).await.unwrap());
        let pauses = crate::storage::pauses::load_all(&pool, 6_000)
            .await
            .unwrap();
        assert_eq!(
            pauses.schedule_setting(Target::Group(group)),
            Assignment::Parent,
            "back to inheriting"
        );
        assert!(list(&pool).await.unwrap().is_empty());
    }
}
