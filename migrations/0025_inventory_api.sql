-- What a device says about itself (`sysDescr`, `sysObjectID`), refreshed by the poller about once a day
-- and by the connection test. Its own table: the `devices` table is rebuilt once by the program
-- (see `storage::migrate`) and would lose new columns. Served by the inventory API (`GET /api/v1/inventory`).
CREATE TABLE device_identity (
    device_id     INTEGER PRIMARY KEY REFERENCES devices (id) ON DELETE CASCADE,
    sys_descr     TEXT NOT NULL,
    sys_object_id TEXT NOT NULL,
    updated_ms    INTEGER NOT NULL
) STRICT;

-- Access tokens of the read-only API. Only the SHA-256 of a token is stored; the token itself is shown
-- once when it is created.
CREATE TABLE api_tokens (
    id            INTEGER PRIMARY KEY,
    name          TEXT NOT NULL COLLATE NOCASE UNIQUE CHECK (length(name) BETWEEN 1 AND 64),
    token_hash    TEXT NOT NULL UNIQUE,
    created_by    TEXT NOT NULL,
    created_ms    INTEGER NOT NULL,
    last_used_ms  INTEGER
) STRICT;
