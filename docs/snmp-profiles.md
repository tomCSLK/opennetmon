# SNMP profiles and encryption

SNMP credentials are stored as reusable **profiles**. A device refers to a profile.
The pages under `/credentials` are only accessible to administrators (viewers get 403).

## Profile types

| Type | Fields |
|---|---|
| SNMPv2c | community |
| SNMPv3 authNoPriv | user, authentication method (MD5, SHA-1, SHA-224/256/384/512), authentication password |
| SNMPv3 authPriv | like authNoPriv, plus encryption method (DES, AES-128/192/256) and encryption password |

For TP-Link Omada devices: **SNMPv3, authNoPriv, MD5**, user as in the Omada controller
(Network Config → General Settings → SNMP).

Rules: name 1–64 characters (unique, case-insensitive), community 1–128 characters, SNMPv3 user 1–32 characters without spaces,
SNMPv3 passwords 8–128 characters (RFC 3414).

## Passwords in the user interface

Passwords and communities are **never** shown, not even masked. The forms only show “set – leave empty to keep”.
When editing, empty password fields stay unchanged; a filled-in field replaces the stored secret.
If you change the version (v2c ↔ v3) or the level from authNoPriv to authPriv, the new secrets have to be entered.
When switching to authNoPriv, the encryption password is deleted.

## Encryption in the database

- Method: XChaCha20-Poly1305 (24-byte random nonce per value). Format of a value: `0x01 || nonce || ciphertext+tag`.
- The context (`snmp.community`, `snmp.auth_password`, `snmp.priv_password`) is bound as additional authenticated data:
  a ciphertext cannot be copied into another column.
- The key (256 bits, 64 hex characters) is in a file whose path is in the config (`secret_key_file`,
  default `/etc/opennetmon/secret.key`, environment variable `OPENNETMON_SECRET_KEY_FILE`).
  The server only starts if the file exists and is **not readable by group or others** (`chmod 600`).
- Generate the key (once; never overwrites an existing file):

  ```bash
  opennetmon --config /etc/opennetmon/config.toml key generate
  ```

- **Back up the key separately from the database.** Without the key, the stored secrets can no longer be read; the profiles would have to be entered again.
  Key rotation is not implemented yet; the format contains a version byte for it.
- Decryption only happens for the poller (`storage::credentials::load_credentials`), never for the web interface.

## Database

Table `credential_profiles` (migration `0003`). A `CHECK` constraint enforces that exactly the matching columns are filled for each version and security
level (e.g. no encryption password with authNoPriv). Deleting a profile that is still used by devices is prevented by the devices table
(`ON DELETE RESTRICT`); the user interface shows a message instead.
