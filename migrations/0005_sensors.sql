-- Sensors, their channels and the raw measurement samples.
CREATE TABLE sensors (
    id         INTEGER PRIMARY KEY,
    device_id  INTEGER NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    name       TEXT NOT NULL COLLATE NOCASE,
    kind       TEXT NOT NULL CHECK (kind IN ('ping', 'uptime', 'if_traffic', 'if_status', 'oid')),
    if_index   INTEGER,
    oid        TEXT,
    oid_mode   TEXT CHECK (oid_mode IN ('gauge', 'counter')),
    scale      REAL,
    enabled    INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (device_id, name),
    -- Exactly the columns that belong to the kind are set.
    CHECK (
        CASE kind
            WHEN 'if_traffic' THEN if_index IS NOT NULL AND oid IS NULL AND oid_mode IS NULL AND scale IS NULL
            WHEN 'if_status'  THEN if_index IS NOT NULL AND oid IS NULL AND oid_mode IS NULL AND scale IS NULL
            WHEN 'oid'        THEN if_index IS NULL AND oid IS NOT NULL AND oid_mode IS NOT NULL AND scale IS NOT NULL
            ELSE if_index IS NULL AND oid IS NULL AND oid_mode IS NULL AND scale IS NULL
        END
    )
) STRICT;

CREATE INDEX sensors_device_id ON sensors (device_id);

CREATE TABLE channels (
    id        INTEGER PRIMARY KEY,
    sensor_id INTEGER NOT NULL REFERENCES sensors (id) ON DELETE CASCADE,
    key       TEXT NOT NULL,
    unit      TEXT NOT NULL,
    UNIQUE (sensor_id, key)
) STRICT;

-- ts is milliseconds since the Unix epoch (UTC).
CREATE TABLE samples (
    channel_id INTEGER NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    ts         INTEGER NOT NULL,
    value      REAL NOT NULL,
    PRIMARY KEY (channel_id, ts)
) STRICT, WITHOUT ROWID;
