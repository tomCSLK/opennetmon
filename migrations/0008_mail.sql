-- E-mail notification settings (a single row) and the log of sent notifications.
CREATE TABLE mail_settings (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    enabled         INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    host            TEXT NOT NULL DEFAULT '',
    port            INTEGER NOT NULL DEFAULT 587 CHECK (port BETWEEN 1 AND 65535),
    security        TEXT NOT NULL DEFAULT 'starttls' CHECK (security IN ('none', 'starttls', 'tls')),
    username        TEXT,
    password_enc    BLOB,
    from_address    TEXT NOT NULL DEFAULT '',
    recipients      TEXT NOT NULL DEFAULT '',
    notify_warnings INTEGER NOT NULL DEFAULT 1 CHECK (notify_warnings IN (0, 1)),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- A password only makes sense together with a user name.
    CHECK (password_enc IS NULL OR username IS NOT NULL)
) STRICT;

INSERT INTO mail_settings (id) VALUES (1);

CREATE TABLE notifications (
    id         INTEGER PRIMARY KEY,
    ts_ms      INTEGER NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('alert', 'test')),
    subject    TEXT NOT NULL,
    recipients TEXT NOT NULL,
    ok         INTEGER NOT NULL CHECK (ok IN (0, 1)),
    error      TEXT
) STRICT;

CREATE INDEX notifications_ts ON notifications (ts_ms);
