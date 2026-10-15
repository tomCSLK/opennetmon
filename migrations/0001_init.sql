-- Phase 1 baseline: key/value table for schema-level metadata.
-- Domain tables (groups, devices, sensors, samples, ...) follow with their MVP steps.
CREATE TABLE meta (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;

INSERT INTO meta (key, value) VALUES ('created_at', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
