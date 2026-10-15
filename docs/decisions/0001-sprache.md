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
| lokaler `snmpd` | v2c | – | offen |
| lokaler `snmpd` | v3 authPriv | SHA-256 / AES-128 | offen |
| EXOS-Gerät | v2c | – | offen |
| EXOS-Gerät | v3 authPriv | SHA-256 / AES-128 | offen |
| EXOS-Gerät | v3 authPriv | konfigurierte Kombination: _noch zu klären_ | offen |
| VOSS-Gerät | v2c | – | offen |
| VOSS-Gerät | v3 authPriv | SHA-256 / AES-128 | offen |
| VOSS-Gerät | v3 authPriv | konfigurierte Kombination: _noch zu klären_ | offen |
| alle | GETBULK-Walk `ifTable`/`ifXTable` | – | offen |

## Entscheidung

_Wird nach dem Spike eingetragen._ Fallback auf Python (FastAPI, `pysnmp` 7.x) nur, wenn SNMPv3 mit der
konfigurierten Kombination mit `snmp2` nicht zuverlässig läuft und sich nicht mit vertretbarem Aufwand lösen lässt.
Bei Problemen wird der Befund hier dokumentiert und nachgefragt, die Sprache wird nicht eigenmächtig gewechselt.

## Konsequenzen

_Wird nach dem Spike eingetragen._
