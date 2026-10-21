-- Pauses like PRTG: a paused group, device or sensor (and everything below it) is not polled and sends
-- no notifications. A sensor's own pause stays `sensors.enabled = 0`; devices and groups get the same
-- flag. `pause_until_ms` NULL = until resumed by hand; the reason is optional.
ALTER TABLE sensors ADD COLUMN pause_until_ms INTEGER;
ALTER TABLE sensors ADD COLUMN pause_reason TEXT NOT NULL DEFAULT '';
ALTER TABLE devices ADD COLUMN paused INTEGER NOT NULL DEFAULT 0 CHECK (paused IN (0, 1));
ALTER TABLE devices ADD COLUMN pause_until_ms INTEGER;
ALTER TABLE devices ADD COLUMN pause_reason TEXT NOT NULL DEFAULT '';
ALTER TABLE groups ADD COLUMN paused INTEGER NOT NULL DEFAULT 0 CHECK (paused IN (0, 1));
ALTER TABLE groups ADD COLUMN pause_until_ms INTEGER;
ALTER TABLE groups ADD COLUMN pause_reason TEXT NOT NULL DEFAULT '';

-- Who paused or resumed what, and when a pause ran out (user_name NULL); shown on the log page.
CREATE TABLE pause_log (
    id        INTEGER PRIMARY KEY,
    ts_ms     INTEGER NOT NULL,
    action    TEXT NOT NULL CHECK (action IN ('paused', 'resumed', 'expired')),
    sensor_id INTEGER REFERENCES sensors (id) ON DELETE CASCADE,
    device_id INTEGER REFERENCES devices (id) ON DELETE CASCADE,
    group_id  INTEGER REFERENCES groups (id) ON DELETE CASCADE,
    until_ms  INTEGER,
    reason    TEXT NOT NULL DEFAULT '',
    user_name TEXT,
    CHECK ((sensor_id IS NOT NULL) + (device_id IS NOT NULL) + (group_id IS NOT NULL) = 1)
) STRICT;

CREATE INDEX pause_log_ts ON pause_log (ts_ms);
CREATE INDEX pause_log_sensor ON pause_log (sensor_id);
CREATE INDEX pause_log_device ON pause_log (device_id);
CREATE INDEX pause_log_group ON pause_log (group_id);

-- Disabled sensors kept their last state, so resuming one that was down sent a stale recovery mail.
-- Paused is the clean start: the first poll after resuming is treated like the very first.
UPDATE sensor_status SET status = 'paused', consecutive_failures = 0
 WHERE sensor_id IN (SELECT id FROM sensors WHERE enabled = 0);
