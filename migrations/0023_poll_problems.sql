-- Polls that did not end "up" (warning or down, with the message), so the table of values can show
-- failed polls like PRTG instead of leaving them out: a failed poll writes no samples. Kept like the
-- raw samples (`retention_raw_days`).
CREATE TABLE poll_problems (
    sensor_id INTEGER NOT NULL REFERENCES sensors (id) ON DELETE CASCADE,
    ts        INTEGER NOT NULL,
    status    TEXT NOT NULL CHECK (status IN ('warning', 'down', 'unknown')),
    message   TEXT NOT NULL,
    PRIMARY KEY (sensor_id, ts)
) STRICT, WITHOUT ROWID;

CREATE INDEX poll_problems_ts ON poll_problems (ts);
