-- Device templates made from a configured device (rules as JSON, see src/templates.rs) and the
-- template a device was last set up with.
CREATE TABLE device_templates (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL COLLATE NOCASE UNIQUE,
    -- sysObjectID of the source device: devices reporting the same one get this template suggested.
    object_id  TEXT,
    -- Name of the device the template was made from (informational).
    source     TEXT NOT NULL DEFAULT '',
    body       TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

ALTER TABLE devices ADD COLUMN template_id INTEGER REFERENCES device_templates (id) ON DELETE SET NULL;
