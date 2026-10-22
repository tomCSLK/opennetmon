# 0001 – Programming language

- **Status:** decided: Rust (confirmed on 2026-10-15, after a successful spike against real Omada devices)
- **Preliminary decision:** Rust (according to `CLAUDE.md`), Python only as a fallback
- **Date:** 2026-10-15

## Context

`opennetmon` is to monitor TP-Link Omada devices (ER605, SG2008/SG2008P, EAP225/EAP653) via SNMP. Rust is preferred (static binary, low resource usage).
The risk lies in SNMPv3 with the auth/priv combination configured on the devices and in the library `snmp2`.

## Spike: `examples/snmp_probe`

It queries `sysDescr`, `sysUpTime` and `IF-MIB::ifHCInOctets` as well as a GETBULK walk over `ifTable`/`ifXTable`.

| Target | Protocol | Auth/priv | Result |
|---|---|---|---|
| local `snmpd` | v2c | – | ok: `sysDescr`/`sysUpTime`. Walks empty, because Ubuntu restricts the default community `public` to the system group (same behaviour with `snmpbulkwalk`) |
| local `snmpd` | v3 authPriv | SHA-256 / AES-128 | ok: `sysDescr`, `sysUpTime`, `ifHCInOctets` (3 interfaces), GETBULK walk `ifXTable` 54 and `ifTable` 66 varbinds, identical to net-snmp |
| switch SG2008 (two devices) | v3 **authNoPriv** | MD5 | ok: `sysDescr`, `sysUpTime`, `ifHCInOctets` (9 interfaces), `ifXTable` 171 and `ifTable` 198 varbinds |
| switch SG2008P | v3 **authNoPriv** | MD5 | ok, as above |
| AP EAP653 (two devices) | v3 authNoPriv | MD5 | ok: `sysDescr`, `sysUpTime`, `ifTable` (286/308 varbinds). **No `ifXTable`, no 64-bit counters** |
| AP EAP225 | v3 authNoPriv | MD5 | ok, like the EAP653 (`ifTable` 198 varbinds, no `ifXTable`) |
| gateway ER605 | v3 | – | no answer (`Receive` error during engine discovery); probably no SNMP agent |
| switch SG2008 | v3 authPriv | all combinations of MD5/SHA/SHA-256 and DES/AES | rejected: the device only supports authNoPriv |

## Results (2026-10-15)

- `snmp2` 0.5.2 with `tokio` + `crypto-rust` (without OpenSSL) builds and runs; v3 SHA-256/AES-128 against net-snmp 5.9.4 works.
- A wrong auth password or a wrong auth protocol leads to `AuthFailure(NotAuthenticated)`, an unreachable agent to `Receive`.
- `snmp2` has no built-in timeout (`recv` waits forever); the caller must limit every request with `tokio::time::timeout`.
- **Omada SNMP is only v3 with authNoPriv and MD5** (one user, one password, no encryption password). Authentication without encryption is acceptable for a
  private network; the controller offers nothing stronger. The credential profile must therefore support `authNoPriv` and MD5.
- **Switches** return `ifXTable` with 64-bit counters. **Access points** only return `ifTable` with 32-bit counters; at 1 Gbit/s a 32-bit counter wraps in
  about 34 seconds, so the rate sensor must handle wraps at 32 bits cleanly and the interval must be chosen accordingly (or AP traffic left out at first).
- The **ER605** does not answer SNMP and drops out of the SNMP MVP for now (a ping sensor stays possible).
- The switches' interface indexes are not consecutive (e.g. 49153, 49154 for trunks/LAGs); discovery must evaluate `ifDescr`/`ifName`.

## Decision

**Decision: Rust stays.** `snmp2` with `crypto-rust` works reliably against all SNMP-capable Omada devices (v3 authNoPriv/MD5) and against the local `snmpd`
with SHA-256/AES-128. Fall back to Python (FastAPI, `pysnmp` 7.x) only if SNMPv3 with the configured combination does not run reliably with `snmp2` and
cannot be fixed with reasonable effort. Problems are documented here and discussed; the language is not switched unilaterally.

## Consequences

Continue with Rust according to `CLAUDE.md`. Adjustments: credential profiles with `authNoPriv`/MD5; handling of 32-bit counters; interface names via
`ifName`/`ifDescr`.
