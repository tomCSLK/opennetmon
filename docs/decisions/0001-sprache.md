# 0001 – Programmiersprache

- **Status:** Offen (Phase-0-Spike steht aus)
- **Vorläufige Entscheidung:** Rust (laut `CLAUDE.md`), Python nur als Fallback
- **Datum:** 2026-10-15

## Kontext

`netmon` soll EXOS- und VOSS-Geräte per SNMP überwachen. Rust ist bevorzugt (statisches Binary, geringer
Ressourcenbedarf). Das Risiko liegt in SNMPv3 mit der auf den Geräten konfigurierten Auth/Priv-Kombination
und der Bibliothek `snmp2`.

## Spike: `examples/snmp_probe`

Abgefragt werden `sysDescr`, `sysUpTime` und `IF-MIB::ifHCInOctets` sowie ein GETBULK-Walk über
`ifTable`/`ifXTable`.

| Ziel | Protokoll | Auth/Priv | Ergebnis |
|---|---|---|---|
| lokaler `snmpd` | v2c | – | ok: `sysDescr`/`sysUpTime`. Walks leer, da die Standard-Community `public` auf Ubuntu auf die Systemgruppe beschränkt ist (gleiches Verhalten mit `snmpbulkwalk`) |
| lokaler `snmpd` | v3 authPriv | SHA-256 / AES-128 | ok: `sysDescr`, `sysUpTime`, `ifHCInOctets` (3 Interfaces), GETBULK-Walk `ifXTable` 54 und `ifTable` 66 Varbinds, identisch zu net-snmp |
| EXOS-Gerät | v2c | – | offen |
| EXOS-Gerät | v3 authPriv | SHA-256 / AES-128 | offen |
| EXOS-Gerät | v3 authPriv | konfigurierte Kombination: _noch zu klären_ | offen |
| VOSS-Gerät | v2c | – | offen |
| VOSS-Gerät | v3 authPriv | SHA-256 / AES-128 | offen |
| VOSS-Gerät | v3 authPriv | konfigurierte Kombination: _noch zu klären_ | offen |
| alle | GETBULK-Walk `ifTable`/`ifXTable` | – | offen |

## Zwischenstand (2026-10-15, nur lokaler Agent)

- `snmp2` 0.5.2 mit `tokio` + `crypto-rust` (ohne OpenSSL) baut und läuft; v3 SHA-256/AES-128 gegen net-snmp 5.9.4 funktioniert.
- Falsches Auth-Passwort oder falsches Auth-Protokoll führt zu `AuthFailure(NotAuthenticated)`, ein nicht erreichbarer Agent zu `Receive`.
- `snmp2` hat keinen eingebauten Timeout (`recv` wartet unbegrenzt), der Aufrufer muss jede Anfrage selbst mit `tokio::time::timeout` begrenzen.
- Noch offen und entscheidend: die Geräte (EXOS/VOSS) mit der dort konfigurierten Auth/Priv-Kombination. Ohne diese Tests keine Entscheidung.

## Entscheidung

_Wird nach dem Spike eingetragen._ Fallback auf Python (FastAPI, `pysnmp` 7.x) nur, wenn SNMPv3 mit der
konfigurierten Kombination mit `snmp2` nicht zuverlässig läuft und sich nicht mit vertretbarem Aufwand lösen lässt.
Bei Problemen wird der Befund hier dokumentiert und nachgefragt, die Sprache wird nicht eigenmächtig gewechselt.

## Konsequenzen

_Wird nach dem Spike eingetragen._
