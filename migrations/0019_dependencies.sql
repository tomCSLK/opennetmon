-- Dependencies like PRTG: a group, device or sensor is paused while another sensor is down (or paused),
-- e.g. the switches while the ping of the router fails. `dependency`: 'parent' = take it from the object
-- above (the default), 'sensor' = depend on `dependency_sensor_id`, 'none' = no dependency (ends the
-- inheritance). A deleted sensor sets the reference to NULL, which reads as 'parent' again.
-- `dependency_delay_secs`: how long the objects stay paused after that sensor is fine again (NULL = take
-- it from the object above, default 0). A master sensor pauses the other sensors of its device.
-- `sensors` is also defined in `storage::allow_local_sensors` and `devices` in
-- `storage::allow_devices_without_profile`: new columns go there as well.
ALTER TABLE groups ADD COLUMN dependency TEXT NOT NULL DEFAULT 'parent' CHECK (dependency IN ('parent', 'sensor', 'none'));
ALTER TABLE groups ADD COLUMN dependency_sensor_id INTEGER REFERENCES sensors (id) ON DELETE SET NULL;
ALTER TABLE groups ADD COLUMN dependency_delay_secs INTEGER CHECK (dependency_delay_secs IS NULL OR dependency_delay_secs BETWEEN 0 AND 3600);
ALTER TABLE devices ADD COLUMN dependency TEXT NOT NULL DEFAULT 'parent' CHECK (dependency IN ('parent', 'sensor', 'none'));
ALTER TABLE devices ADD COLUMN dependency_sensor_id INTEGER REFERENCES sensors (id) ON DELETE SET NULL;
ALTER TABLE devices ADD COLUMN dependency_delay_secs INTEGER CHECK (dependency_delay_secs IS NULL OR dependency_delay_secs BETWEEN 0 AND 3600);
ALTER TABLE sensors ADD COLUMN dependency TEXT NOT NULL DEFAULT 'parent' CHECK (dependency IN ('parent', 'sensor', 'none'));
ALTER TABLE sensors ADD COLUMN dependency_sensor_id INTEGER REFERENCES sensors (id) ON DELETE SET NULL;
ALTER TABLE sensors ADD COLUMN dependency_delay_secs INTEGER CHECK (dependency_delay_secs IS NULL OR dependency_delay_secs BETWEEN 0 AND 3600);
ALTER TABLE sensors ADD COLUMN dependency_master INTEGER NOT NULL DEFAULT 0 CHECK (dependency_master IN (0, 1));

-- When a sensor started and stopped pausing other objects through dependencies (one entry each instead
-- of one per paused sensor); `sensors`/`devices` count what it paused. Shown on the log page.
CREATE TABLE dependency_log (
    id        INTEGER PRIMARY KEY,
    ts_ms     INTEGER NOT NULL,
    sensor_id INTEGER NOT NULL REFERENCES sensors (id) ON DELETE CASCADE,
    action    TEXT NOT NULL CHECK (action IN ('active', 'ended')),
    sensors   INTEGER NOT NULL DEFAULT 0,
    devices   INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX dependency_log_ts ON dependency_log (ts_ms);
CREATE INDEX dependency_log_sensor ON dependency_log (sensor_id);
