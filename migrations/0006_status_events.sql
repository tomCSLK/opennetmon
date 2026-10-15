-- Current status per sensor and the log of status changes.
-- Timestamps are milliseconds since the Unix epoch (UTC).
CREATE TABLE sensor_status (
    sensor_id            INTEGER PRIMARY KEY REFERENCES sensors (id) ON DELETE CASCADE,
    status               TEXT NOT NULL CHECK (status IN ('up', 'warning', 'down', 'unknown', 'paused')),
    since_ms             INTEGER NOT NULL,
    last_poll_ms         INTEGER NOT NULL,
    message              TEXT NOT NULL,
    consecutive_failures INTEGER NOT NULL DEFAULT 0 CHECK (consecutive_failures >= 0)
) STRICT;

CREATE TABLE events (
    id          INTEGER PRIMARY KEY,
    ts_ms       INTEGER NOT NULL,
    sensor_id   INTEGER NOT NULL REFERENCES sensors (id) ON DELETE CASCADE,
    from_status TEXT CHECK (from_status IN ('up', 'warning', 'down', 'unknown', 'paused')),
    to_status   TEXT NOT NULL CHECK (to_status IN ('up', 'warning', 'down', 'unknown', 'paused')),
    message     TEXT NOT NULL
) STRICT;

CREATE INDEX events_ts ON events (ts_ms);
CREATE INDEX events_sensor ON events (sensor_id, ts_ms);
