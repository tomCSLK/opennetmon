-- Per-channel thresholds (base unit of the channel) and the measurement rollups.
ALTER TABLE channels ADD COLUMN warn_above REAL;
ALTER TABLE channels ADD COLUMN warn_below REAL;
ALTER TABLE channels ADD COLUMN down_above REAL;
ALTER TABLE channels ADD COLUMN down_below REAL;

-- ts is the start of the hour (UTC) in milliseconds; n is the number of raw samples.
CREATE TABLE samples_1h (
    channel_id INTEGER NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    ts         INTEGER NOT NULL,
    min        REAL NOT NULL,
    avg        REAL NOT NULL,
    max        REAL NOT NULL,
    n          INTEGER NOT NULL CHECK (n > 0),
    PRIMARY KEY (channel_id, ts)
) STRICT, WITHOUT ROWID;

-- ts is the start of the day (UTC) in milliseconds; avg is weighted by the hourly sample counts.
CREATE TABLE samples_1d (
    channel_id INTEGER NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    ts         INTEGER NOT NULL,
    min        REAL NOT NULL,
    avg        REAL NOT NULL,
    max        REAL NOT NULL,
    n          INTEGER NOT NULL CHECK (n > 0),
    PRIMARY KEY (channel_id, ts)
) STRICT, WITHOUT ROWID;
