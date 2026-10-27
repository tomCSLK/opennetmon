-- Availability like the reports of PRTG: milliseconds per status and hour (`ts` = start of the hour, UTC),
-- counted by every poll for the time since the poll before (gaps longer than three intervals are left out).
-- `availability_1d` sums the hours per UTC day and is kept when the hours are deleted after
-- `retention_hourly_days`.
CREATE TABLE availability_1h (
    sensor_id  INTEGER NOT NULL REFERENCES sensors (id) ON DELETE CASCADE,
    ts         INTEGER NOT NULL,
    up_ms      INTEGER NOT NULL DEFAULT 0,
    warning_ms INTEGER NOT NULL DEFAULT 0,
    down_ms    INTEGER NOT NULL DEFAULT 0,
    unknown_ms INTEGER NOT NULL DEFAULT 0,
    paused_ms  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (sensor_id, ts)
) STRICT, WITHOUT ROWID;

CREATE INDEX availability_1h_ts ON availability_1h (ts);

CREATE TABLE availability_1d (
    sensor_id  INTEGER NOT NULL REFERENCES sensors (id) ON DELETE CASCADE,
    ts         INTEGER NOT NULL,
    up_ms      INTEGER NOT NULL DEFAULT 0,
    warning_ms INTEGER NOT NULL DEFAULT 0,
    down_ms    INTEGER NOT NULL DEFAULT 0,
    unknown_ms INTEGER NOT NULL DEFAULT 0,
    paused_ms  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (sensor_id, ts)
) STRICT, WITHOUT ROWID;
