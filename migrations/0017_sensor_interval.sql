-- A sensor may be polled at its own interval (seconds, like the device's 10 to 3600);
-- NULL = the interval of its device. Requested in GitHub issue #1.
ALTER TABLE sensors ADD COLUMN interval_secs INTEGER CHECK (interval_secs IS NULL OR interval_secs BETWEEN 10 AND 3600);
