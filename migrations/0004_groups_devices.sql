-- Hierarchical device groups and devices.
CREATE TABLE groups (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL,
    parent_id  INTEGER REFERENCES groups (id) ON DELETE RESTRICT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (parent_id IS NULL OR parent_id <> id)
) STRICT;

-- Names are unique per parent (NULL parents are treated as one level), case-insensitively.
CREATE UNIQUE INDEX groups_unique_name ON groups (COALESCE(parent_id, 0), name COLLATE NOCASE);

CREATE TABLE devices (
    id            INTEGER PRIMARY KEY,
    name          TEXT NOT NULL COLLATE NOCASE UNIQUE,
    host          TEXT NOT NULL,
    port          INTEGER NOT NULL DEFAULT 161 CHECK (port BETWEEN 1 AND 65535),
    group_id      INTEGER REFERENCES groups (id) ON DELETE RESTRICT,
    profile_id    INTEGER NOT NULL REFERENCES credential_profiles (id) ON DELETE RESTRICT,
    interval_secs INTEGER NOT NULL DEFAULT 60 CHECK (interval_secs BETWEEN 10 AND 3600),
    timeout_secs  INTEGER NOT NULL DEFAULT 5 CHECK (timeout_secs BETWEEN 1 AND 60),
    retries       INTEGER NOT NULL DEFAULT 1 CHECK (retries BETWEEN 0 AND 5),
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX devices_group_id ON devices (group_id);
CREATE INDEX devices_profile_id ON devices (profile_id);
