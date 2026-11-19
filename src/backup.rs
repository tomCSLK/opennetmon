//! Daily backup made by the server itself (config `backup_dir`, `backup_keep`, `backup_time`), for the
//! container image, which has no systemd timer. It does what `deploy/opennetmon-backup.sh` does for the
//! `.deb`: a consistent copy `opennetmon-YYYYMMDD-HHMMSS.db` (local time), the newest `backup_keep` copies
//! stay. A copy is written as `….db.part` and renamed when complete, so a stop in the middle leaves no
//! file that looks like a backup. Other files in the directory (e.g. `before-0.7.0.db`) are never deleted.
//! Like the timer of the `.deb` (`Persistent=true`), a backup that was missed while the server was down is
//! made a few minutes after the start.

use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, Zoned};
use sqlx::SqlitePool;

use crate::error::{Error, Result};

const PREFIX: &str = "opennetmon-";
const SUFFIX: &str = ".db";
const PARTIAL: &str = ".part";
/// A backup older than this at the start counts as missed.
const MISSED_AFTER: Duration = Duration::from_secs(25 * 3600);
/// Wait after the start before a missed backup is made (the server settles first).
const CATCH_UP_DELAY: Duration = Duration::from_secs(5 * 60);

/// Where, how many and when.
#[derive(Debug, Clone)]
pub struct Plan {
    pub dir: PathBuf,
    pub keep: u32,
    /// Minutes after local midnight.
    pub minute: u16,
}

/// Runs the backups until the task is aborted.
pub fn spawn(pool: SqlitePool, plan: Plan, tz: TimeZone) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if missed(&plan.dir, SystemTime::now()) {
            tracing::info!(dir = %plan.dir.display(), "no recent backup; making one in five minutes");
            tokio::time::sleep(CATCH_UP_DELAY).await;
            run_logged(&pool, &plan, &tz).await;
        }
        loop {
            let next = next_run(Timestamp::now(), plan.minute, &tz);
            let wait = Timestamp::now()
                .duration_until(next)
                .unsigned_abs()
                .min(Duration::from_secs(2 * 24 * 3600));
            tokio::time::sleep(wait).await;
            run_logged(&pool, &plan, &tz).await;
        }
    })
}

async fn run_logged(pool: &SqlitePool, plan: &Plan, tz: &TimeZone) {
    match run(pool, plan, &Timestamp::now().to_zoned(tz.clone())).await {
        Ok((path, removed)) => {
            tracing::info!(path = %path.display(), removed, "backup written");
        }
        Err(e) => tracing::warn!(dir = %plan.dir.display(), error = %e, "backup failed"),
    }
}

/// The first time after `now` that is `minute` minutes after local midnight in `tz`. On the day the
/// clocks go forward a time in the gap moves forward with them (jiff's "compatible" rule).
pub fn next_run(now: Timestamp, minute: u16, tz: &TimeZone) -> Timestamp {
    let today = now.to_zoned(tz.clone()).date();
    let at = |date: jiff::civil::Date| {
        date.at(
            i8::try_from(minute / 60).unwrap_or(0),
            i8::try_from(minute % 60).unwrap_or(0),
            0,
            0,
        )
        .to_zoned(tz.clone())
        .map(|z| z.timestamp())
    };
    match at(today) {
        Ok(time) if time > now => time,
        _ => today
            .checked_add(1.day())
            .ok()
            .and_then(|tomorrow| at(tomorrow).ok())
            .unwrap_or_else(|| now + 1.day()),
    }
}

/// Writes one backup named after `now` into the directory and prunes old ones; returns the new file and
/// how many old backups were deleted.
pub async fn run(pool: &SqlitePool, plan: &Plan, now: &Zoned) -> Result<(PathBuf, usize)> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&plan.dir)?;
    remove_partial(&plan.dir)?;
    let name = format!("{PREFIX}{}{SUFFIX}", now.strftime("%Y%m%d-%H%M%S"));
    let target = plan.dir.join(&name);
    let partial = plan.dir.join(format!("{name}{PARTIAL}"));
    if target.try_exists()? {
        return Err(Error::invalid(
            crate::i18n::Msg::new("backup-exists").with("path", target.display().to_string()),
        ));
    }
    crate::storage::backup_to(pool, &partial).await?;
    std::fs::rename(&partial, &target)?;
    let removed = prune(&plan.dir, plan.keep)?;
    Ok((target, removed))
}

/// Whether `name` is one of ours: `opennetmon-YYYYMMDD-HHMMSS.db`.
fn is_backup(name: &str) -> bool {
    let Some(stamp) = name
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.strip_suffix(SUFFIX))
    else {
        return false;
    };
    let bytes = stamp.as_bytes();
    bytes.len() == 15
        && bytes[8] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 8 || b.is_ascii_digit())
}

/// Our backups in the directory, newest first (the names sort by time).
fn backups(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut found: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_name().to_str().is_some_and(is_backup) && entry.file_type()?.is_file() {
            found.push(entry.path());
        }
    }
    found.sort_unstable_by(|a, b| b.cmp(a));
    Ok(found)
}

/// Deletes all but the newest `keep` backups; returns how many were deleted.
fn prune(dir: &Path, keep: u32) -> Result<usize> {
    let old = backups(dir)?
        .into_iter()
        .skip(usize::try_from(keep).unwrap_or(usize::MAX));
    let mut removed = 0;
    for path in old {
        std::fs::remove_file(&path)?;
        removed += 1;
    }
    Ok(removed)
}

/// Removes what an interrupted backup left behind.
fn remove_partial(dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let leftover = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_suffix(PARTIAL))
            .is_some_and(is_backup);
        if leftover {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Whether the newest backup is older than [`MISSED_AFTER`] (or there is none) at `now`.
fn missed(dir: &Path, now: SystemTime) -> bool {
    let newest = backups(dir)
        .ok()
        .and_then(|found| found.into_iter().next())
        .and_then(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok());
    newest.is_none_or(|time| now.duration_since(time).is_ok_and(|age| age > MISSED_AFTER))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn berlin() -> TimeZone {
        TimeZone::get("Europe/Berlin").unwrap()
    }

    fn ts(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn the_next_run_is_today_or_tomorrow_in_local_time() {
        let tz = berlin();
        // 03:30 in Berlin is 01:30 UTC in summer, 02:30 UTC in winter.
        assert_eq!(
            next_run(ts("2026-07-01T00:00:00Z"), 210, &tz),
            ts("2026-07-01T01:30:00Z")
        );
        assert_eq!(
            next_run(ts("2026-07-01T01:30:00Z"), 210, &tz),
            ts("2026-07-02T01:30:00Z")
        );
        assert_eq!(
            next_run(ts("2026-12-01T12:00:00Z"), 210, &tz),
            ts("2026-12-02T02:30:00Z")
        );
        // 2027-03-28: 02:00 jumps to 03:00; 02:30 does not exist and moves forward to 03:30 (01:30 UTC).
        assert_eq!(
            next_run(ts("2027-03-27T12:00:00Z"), 150, &tz),
            ts("2027-03-28T01:30:00Z")
        );
    }

    #[test]
    fn only_our_names_count_as_backups() {
        assert!(is_backup("opennetmon-20261119-033000.db"));
        for other in [
            "opennetmon-20261119-033000.db.part",
            "opennetmon-2026111-033000.db",
            "opennetmon-20261119_033000.db",
            "before-0.7.0.db",
            "opennetmon.db",
        ] {
            assert!(!is_backup(other), "{other}");
        }
    }

    #[tokio::test]
    async fn backups_are_written_and_the_oldest_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let backup_dir = dir.path().join("backups");
        let pool = crate::storage::connect(&dir.path().join("db.sqlite"))
            .await
            .unwrap();
        crate::storage::migrate(&pool).await.unwrap();
        let plan = Plan {
            dir: backup_dir.clone(),
            keep: 2,
            minute: 210,
        };
        std::fs::create_dir_all(&backup_dir).unwrap();
        std::fs::write(backup_dir.join("before-0.7.0.db"), b"mine").unwrap();
        std::fs::write(backup_dir.join("opennetmon-20000101-000000.db.part"), b"").unwrap();

        let tz = berlin();
        let mut written = Vec::new();
        for day in 1..=3 {
            let now = ts(&format!("2026-11-0{day}T02:30:00Z")).to_zoned(tz.clone());
            let (path, _) = run(&pool, &plan, &now).await.unwrap();
            written.push(path);
        }
        assert_eq!(
            written[2].file_name().unwrap(),
            "opennetmon-20261103-033000.db"
        );
        let mut names: Vec<String> = std::fs::read_dir(&backup_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "before-0.7.0.db",
                "opennetmon-20261102-033000.db",
                "opennetmon-20261103-033000.db"
            ]
        );
        // A copy is a database of its own.
        let copy = crate::storage::connect(&written[2]).await.unwrap();
        crate::storage::ping(&copy).await.unwrap();

        // The same second twice is refused instead of overwriting.
        let again = ts("2026-11-03T02:30:00Z").to_zoned(tz);
        assert!(run(&pool, &plan, &again).await.is_err());

        assert!(!missed(&backup_dir, SystemTime::now()));
        let later = SystemTime::now() + Duration::from_secs(26 * 3600);
        assert!(missed(&backup_dir, later));
        assert!(missed(&dir.path().join("nothing"), SystemTime::now()));
    }
}
