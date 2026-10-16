-- Uploaded MIB files (the bundled standard MIBs live in the binary) and an optional unit label for
-- custom OID sensors (e.g. taken from the MIB's UNITS clause).
CREATE TABLE mib_files (
    id          INTEGER PRIMARY KEY,
    file_name   TEXT NOT NULL,
    -- Names of the modules defined in the file, separated by spaces.
    modules     TEXT NOT NULL,
    content     TEXT NOT NULL,
    size        INTEGER NOT NULL CHECK (size >= 0),
    uploaded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

ALTER TABLE sensors ADD COLUMN unit_label TEXT;
