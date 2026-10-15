-- Reusable SNMP credential profiles. Secrets are stored encrypted (see src/crypto.rs);
-- the CHECK constraint keeps the column combination consistent per SNMP version/level.
CREATE TABLE credential_profiles (
    id                INTEGER PRIMARY KEY,
    name              TEXT NOT NULL COLLATE NOCASE UNIQUE,
    version           TEXT NOT NULL CHECK (version IN ('v2c', 'v3')),
    community_enc     BLOB,
    username          TEXT,
    security_level    TEXT CHECK (security_level IN ('auth_no_priv', 'auth_priv')),
    auth_protocol     TEXT CHECK (auth_protocol IN ('md5', 'sha1', 'sha224', 'sha256', 'sha384', 'sha512')),
    auth_password_enc BLOB,
    priv_protocol     TEXT CHECK (priv_protocol IN ('des', 'aes128', 'aes192', 'aes256')),
    priv_password_enc BLOB,
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (
        (version = 'v2c'
            AND community_enc IS NOT NULL
            AND username IS NULL AND security_level IS NULL AND auth_protocol IS NULL
            AND auth_password_enc IS NULL AND priv_protocol IS NULL AND priv_password_enc IS NULL)
        OR
        (version = 'v3'
            AND community_enc IS NULL
            AND username IS NOT NULL AND security_level IS NOT NULL
            AND auth_protocol IS NOT NULL AND auth_password_enc IS NOT NULL
            AND (
                (security_level = 'auth_no_priv' AND priv_protocol IS NULL AND priv_password_enc IS NULL)
                OR
                (security_level = 'auth_priv' AND priv_protocol IS NOT NULL AND priv_password_enc IS NOT NULL)
            ))
    )
) STRICT;
