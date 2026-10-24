-- Sign-out after a time without activity (config `session_idle_minutes`): the last activity of a session
-- (Unix seconds, UTC; existing sessions count as active now) and the exception per user, e.g. for a wall display
-- that is never touched and only reloads by itself.
ALTER TABLE sessions ADD COLUMN last_seen_at INTEGER NOT NULL DEFAULT 0;
UPDATE sessions SET last_seen_at = CAST(strftime('%s', 'now') AS INTEGER);
ALTER TABLE users ADD COLUMN stay_signed_in INTEGER NOT NULL DEFAULT 0 CHECK (stay_signed_in IN (0, 1));
