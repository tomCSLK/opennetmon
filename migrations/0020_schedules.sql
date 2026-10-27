-- Maintenance windows like the schedules of PRTG: named schedules with windows, either weekly (on the
-- weekdays of the bit mask `weekdays`, Monday = 1, Tuesday = 2, …, Sunday = 64, from `start_minute`
-- after midnight in the time zone of the server for `duration_minutes`) or once (`start_ms` to `end_ms`, UTC).
-- During a window the objects the schedule is assigned to are paused, computed like a dependency and
-- not stored as a pause.
CREATE TABLE schedules (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL COLLATE NOCASE UNIQUE,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE schedule_windows (
    id               INTEGER PRIMARY KEY,
    schedule_id      INTEGER NOT NULL REFERENCES schedules (id) ON DELETE CASCADE,
    kind             TEXT NOT NULL CHECK (kind IN ('weekly', 'once')),
    weekdays         INTEGER CHECK (weekdays IS NULL OR weekdays BETWEEN 1 AND 127),
    start_minute     INTEGER CHECK (start_minute IS NULL OR start_minute BETWEEN 0 AND 1439),
    duration_minutes INTEGER CHECK (duration_minutes IS NULL OR duration_minutes BETWEEN 1 AND 10080),
    start_ms         INTEGER,
    end_ms           INTEGER,
    CHECK (
        CASE kind
            WHEN 'weekly' THEN weekdays IS NOT NULL AND start_minute IS NOT NULL AND duration_minutes IS NOT NULL
                AND start_ms IS NULL AND end_ms IS NULL
            ELSE weekdays IS NULL AND start_minute IS NULL AND duration_minutes IS NULL
                AND start_ms IS NOT NULL AND end_ms > start_ms
        END
    )
) STRICT;

CREATE INDEX schedule_windows_schedule ON schedule_windows (schedule_id);

-- Which schedule applies to a group, device or sensor: no row = like the object above, a row without
-- `schedule_id` = none (ends the inheritance). Deleting a schedule deletes its rows (back to inheriting).
-- A table of its own instead of columns, so `sensors` needs no rebuild.
CREATE TABLE schedule_assignments (
    id          INTEGER PRIMARY KEY,
    group_id    INTEGER UNIQUE REFERENCES groups (id) ON DELETE CASCADE,
    device_id   INTEGER UNIQUE REFERENCES devices (id) ON DELETE CASCADE,
    sensor_id   INTEGER UNIQUE REFERENCES sensors (id) ON DELETE CASCADE,
    schedule_id INTEGER REFERENCES schedules (id) ON DELETE CASCADE,
    CHECK ((group_id IS NOT NULL) + (device_id IS NOT NULL) + (sensor_id IS NOT NULL) = 1)
) STRICT;

-- When a window of a schedule started and ended pausing objects (one entry each, not one per sensor);
-- `sensors`/`devices` count what it paused. Shown on the log page.
CREATE TABLE schedule_log (
    id          INTEGER PRIMARY KEY,
    ts_ms       INTEGER NOT NULL,
    schedule_id INTEGER NOT NULL REFERENCES schedules (id) ON DELETE CASCADE,
    action      TEXT NOT NULL CHECK (action IN ('started', 'ended')),
    sensors     INTEGER NOT NULL DEFAULT 0,
    devices     INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX schedule_log_ts ON schedule_log (ts_ms);
CREATE INDEX schedule_log_schedule ON schedule_log (schedule_id);
