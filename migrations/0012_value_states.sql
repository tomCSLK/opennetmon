-- Ratings of status values per channel, e.g. PoE status overload(3) = down. The label is the value's
-- name from the MIB, kept for messages (the poller has no MIB index). No rows = values are not rated.
CREATE TABLE value_states (
    channel_id INTEGER NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    value      INTEGER NOT NULL,
    label      TEXT NOT NULL,
    status     TEXT NOT NULL CHECK (status IN ('up', 'warning', 'down')),
    PRIMARY KEY (channel_id, value)
) STRICT, WITHOUT ROWID;
