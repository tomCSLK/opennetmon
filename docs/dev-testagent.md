# Local SNMP test agent (net-snmp)

The local `snmpd` serves as the test agent for the spike and the integration tests (development machine only).
It listens on `127.0.0.1:161`; v2c works with the community `public`.

> Use passwords only locally. Do not write real device credentials into this file or into the repository.

## Creating an SNMPv3 user

Example: user `netmontest`, SHA-256 / AES-128. Choose the passwords freely (at least 8 characters).

```bash
sudo systemctl stop snmpd
echo 'createUser netmontest SHA-256 "AUTH_PASSWORD" AES "PRIV_PASSWORD"' | sudo tee -a /var/lib/snmp/snmpd.conf
echo 'rouser netmontest authPriv' | sudo tee -a /etc/snmp/snmpd.conf
sudo systemctl start snmpd
```

On startup, `snmpd` replaces the `createUser` line in `/var/lib/snmp/snmpd.conf` with the hashed key.
The plain-text password is then no longer in the file. `rouser` allows read-only access exclusively.

## Test

No MIB files are installed on the development machine (`snmp-mibs-downloader` is non-free), so work with numeric OIDs.
The credentials are kept locally in `~/.config/opennetmon-dev/test-agent.env` (permissions 0600, not in the repository).
Load them first with `. ~/.config/opennetmon-dev/test-agent.env`.

```bash
# v2c: sysDescr, sysUpTime
snmpget -v2c -c public -On 127.0.0.1 1.3.6.1.2.1.1.1.0 1.3.6.1.2.1.1.3.0

# v3 authPriv, SHA-256 / AES-128
V3=(-v3 -l authPriv -u "$OPENNETMON_TEST_V3_USER" -a SHA-256 -A "$OPENNETMON_TEST_V3_AUTH_PASS" -x AES -X "$OPENNETMON_TEST_V3_PRIV_PASS")
snmpget "${V3[@]}" -On 127.0.0.1 1.3.6.1.2.1.1.1.0 1.3.6.1.2.1.1.3.0

# GETBULK walk over ifXTable (1.3.6.1.2.1.31.1.1) and ifTable (1.3.6.1.2.1.2.2)
snmpbulkwalk "${V3[@]}" -On 127.0.0.1 1.3.6.1.2.1.31.1.1
snmpbulkwalk "${V3[@]}" -On 127.0.0.1 1.3.6.1.2.1.2.2
```

Expected: v2c and v3 return the same `sysDescr`. `Authentication failure` means a wrong password,
`Unknown user name` that the user was not created (order: first `stop`, then `createUser`, then `start`).
An `snmpset` must be rejected with `noAccess` (only `rouser`).

As of 2026-10-15: v2c, v3 SHA-256/AES-128, GETBULK walk (`ifXTable` 54 rows, `ifTable` 66 rows), a wrong password and the rejected SET were verified
against the local `snmpd` with the command line tools. The Rust spike (`snmp2`) was then tested against the same agent (`docs/decisions/0001-language.md`);
`tests/snmp_local.rs` uses it for the integration tests.

## Testing other combinations

To check the combination used on your devices, create the user with other protocols
(for example `SHA` + `AES`, `SHA-512` + `AES`, `MD5` + `DES`). One `createUser` line and one `rouser` per user.
`snmpd --help`/`man snmpd.conf` shows which protocols the local `snmpd` supports.

## Removing the user again

```bash
sudo systemctl stop snmpd
sudo sed -i '/netmontest/d' /var/lib/snmp/snmpd.conf /etc/snmp/snmpd.conf
sudo systemctl start snmpd
```
