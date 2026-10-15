# 0001 – Programmiersprache

- **Status:** Vorschlag Rust, zur Bestätigung durch den Nutzer (Spike gegen echte Omada-Geräte bestanden)
- **Vorläufige Entscheidung:** Rust (laut `CLAUDE.md`), Python nur als Fallback
- **Datum:** 2026-10-15

## Kontext

`netmon` soll TP-Link-Omada-Geräte (ER605, SG2008/SG2008P, EAP225/EAP653) per SNMP überwachen. Rust ist bevorzugt (statisches Binary, geringer
Ressourcenbedarf). Das Risiko liegt in SNMPv3 mit der auf den Geräten konfigurierten Auth/Priv-Kombination
und der Bibliothek `snmp2`.

## Spike: `examples/snmp_probe`

Abgefragt werden `sysDescr`, `sysUpTime` und `IF-MIB::ifHCInOctets` sowie ein GETBULK-Walk über
`ifTable`/`ifXTable`.

| Ziel | Protokoll | Auth/Priv | Ergebnis |
|---|---|---|---|
| lokaler `snmpd` | v2c | – | ok: `sysDescr`/`sysUpTime`. Walks leer, da die Standard-Community `public` auf Ubuntu auf die Systemgruppe beschränkt ist (gleiches Verhalten mit `snmpbulkwalk`) |
| lokaler `snmpd` | v3 authPriv | SHA-256 / AES-128 | ok: `sysDescr`, `sysUpTime`, `ifHCInOctets` (3 Interfaces), GETBULK-Walk `ifXTable` 54 und `ifTable` 66 Varbinds, identisch zu net-snmp |
| Switch SG2008 (.253, .251) | v3 **authNoPriv** | MD5 | ok: `sysDescr`, `sysUpTime`, `ifHCInOctets` (9 Interfaces), `ifXTable` 171 und `ifTable` 198 Varbinds |
| Switch SG2008P (.252) | v3 **authNoPriv** | MD5 | ok, wie oben |
| AP EAP653 (.240, .241) | v3 authNoPriv | MD5 | ok: `sysDescr`, `sysUpTime`, `ifTable` (286/308 Varbinds). **Kein `ifXTable`, keine 64-Bit-Counter** |
| AP EAP225 (.242) | v3 authNoPriv | MD5 | ok, wie EAP653 (`ifTable` 198 Varbinds, kein `ifXTable`) |
| Gateway ER605 (.1) | v3 | – | keine Antwort (`Receive`-Fehler bei der Engine-Discovery); vermutlich kein SNMP-Agent |
| Switch SG2008 (.253) | v3 authPriv | alle Kombinationen aus MD5/SHA/SHA-256 und DES/AES | abgelehnt: Gerät unterstützt nur authNoPriv |

## Ergebnisse (2026-10-15)

- `snmp2` 0.5.2 mit `tokio` + `crypto-rust` (ohne OpenSSL) baut und läuft; v3 SHA-256/AES-128 gegen net-snmp 5.9.4 funktioniert.
- Falsches Auth-Passwort oder falsches Auth-Protokoll führt zu `AuthFailure(NotAuthenticated)`, ein nicht erreichbarer Agent zu `Receive`.
- `snmp2` hat keinen eingebauten Timeout (`recv` wartet unbegrenzt), der Aufrufer muss jede Anfrage selbst mit `tokio::time::timeout` begrenzen.
- **Omada-SNMP ist nur v3 mit authNoPriv und MD5** (ein Benutzer, ein Passwort, kein Verschlüsselungs-Passwort). Authentifizierung ohne Verschlüsselung ist für ein privates Netz vertretbar; der Controller bietet nichts Stärkeres an. Das Credential-Profil muss deshalb `authNoPriv` und MD5 unterstützen.
- **Switches** liefern `ifXTable` mit 64-Bit-Countern. **Access Points** liefern nur `ifTable` mit 32-Bit-Countern; bei 1 Gbit/s läuft ein 32-Bit-Counter in rund 34 Sekunden über, der Ratensensor muss Wraps bei 32 Bit sauber behandeln und das Intervall passend wählen (oder AP-Traffic zunächst auslassen).
- **ER605** antwortet nicht auf SNMP und fällt im SNMP-MVP vorerst aus (Ping-Sensor bleibt möglich).
- Interface-Indizes der Switches sind nicht fortlaufend (z. B. 49153, 49154 für Trunks/LAGs); Discovery muss `ifDescr`/`ifName` auswerten.

## Entscheidung

**Vorschlag: Rust bleibt.** `snmp2` mit `crypto-rust` arbeitet gegen alle SNMP-fähigen Omada-Geräte zuverlässig (v3 authNoPriv/MD5) und gegen den lokalen `snmpd` mit SHA-256/AES-128. Zu bestätigen durch den Nutzer. Fallback auf Python (FastAPI, `pysnmp` 7.x) nur, wenn SNMPv3 mit der
konfigurierten Kombination mit `snmp2` nicht zuverlässig läuft und sich nicht mit vertretbarem Aufwand lösen lässt.
Bei Problemen wird der Befund hier dokumentiert und nachgefragt, die Sprache wird nicht eigenmächtig gewechselt.

## Konsequenzen

Weiterbauen mit Rust gemäß `CLAUDE.md`. Anpassungen: Credential-Profile mit `authNoPriv`/MD5; 32-Bit-Counter-Behandlung; Interface-Namen per `ifName`/`ifDescr`.
