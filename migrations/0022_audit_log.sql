-- Who changed what and when: changes by administrators, sign-ins (also failed ones) and sign-outs.
-- `detail` is a message (`Msg::encode`, e.g. the changed fields with their old and new values; never a secret).
-- `user_name` and `object_name` are copies as of the time, so entries stay readable after renames and
-- deletions. Kept for `retention_audit_days`.
CREATE TABLE audit_log (
    id          INTEGER PRIMARY KEY,
    ts_ms       INTEGER NOT NULL,
    user_name   TEXT NOT NULL,
    address     TEXT,
    action      TEXT NOT NULL,
    kind        TEXT NOT NULL,
    object_id   INTEGER,
    object_name TEXT NOT NULL DEFAULT '',
    detail      TEXT NOT NULL DEFAULT ''
) STRICT;

CREATE INDEX audit_log_ts ON audit_log (ts_ms);
