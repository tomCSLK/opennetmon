# Lokaler SNMP-Testagent (net-snmp)

Der lokale `snmpd` dient als Testagent für den Spike und die Integrationstests (nur Entwicklungsrechner).
Er lauscht auf `127.0.0.1:161`, v2c funktioniert mit der Community `public`.

> Passwörter nur lokal verwenden. Keine echten Gerätezugangsdaten in diese Datei oder ins Repository schreiben.

## SNMPv3-Benutzer anlegen

Beispiel: Benutzer `netmontest`, SHA-256 / AES-128. Die Passwörter (mindestens 8 Zeichen) frei wählen.

```bash
sudo systemctl stop snmpd
echo 'createUser netmontest SHA-256 "AUTH_PASSWORT" AES "PRIV_PASSWORT"' | sudo tee -a /var/lib/snmp/snmpd.conf
echo 'rouser netmontest authPriv' | sudo tee -a /etc/snmp/snmpd.conf
sudo systemctl start snmpd
```

Beim Start ersetzt `snmpd` die `createUser`-Zeile in `/var/lib/snmp/snmpd.conf` durch den gehashten Schlüssel.
Das Klartextpasswort steht danach nicht mehr in der Datei. `rouser` erlaubt ausschließlich lesenden Zugriff.

## Test

Auf dem Entwicklungsrechner sind keine MIB-Dateien installiert (`snmp-mibs-downloader` ist non-free), daher mit
numerischen OIDs arbeiten. Die Zugangsdaten liegen lokal in `~/.config/netmon-dev/test-agent.env` (Rechte 0600,
nicht im Repository). Vorher laden mit `. ~/.config/netmon-dev/test-agent.env`.

```bash
# v2c: sysDescr, sysUpTime
snmpget -v2c -c public -On 127.0.0.1 1.3.6.1.2.1.1.1.0 1.3.6.1.2.1.1.3.0

# v3 authPriv, SHA-256 / AES-128
V3=(-v3 -l authPriv -u "$NETMON_TEST_V3_USER" -a SHA-256 -A "$NETMON_TEST_V3_AUTH_PASS" -x AES -X "$NETMON_TEST_V3_PRIV_PASS")
snmpget "${V3[@]}" -On 127.0.0.1 1.3.6.1.2.1.1.1.0 1.3.6.1.2.1.1.3.0

# GETBULK-Walk über ifXTable (1.3.6.1.2.1.31.1.1) und ifTable (1.3.6.1.2.1.2.2)
snmpbulkwalk "${V3[@]}" -On 127.0.0.1 1.3.6.1.2.1.31.1.1
snmpbulkwalk "${V3[@]}" -On 127.0.0.1 1.3.6.1.2.1.2.2
```

Erwartung: v2c und v3 liefern dieselbe `sysDescr`. `Authentication failure` bedeutet falsches Passwort,
`Unknown user name`, dass der Benutzer nicht angelegt wurde (Reihenfolge: erst `stop`, dann `createUser`, dann `start`).
Ein `snmpset` muss mit `noAccess` abgelehnt werden (nur `rouser`).

Stand 2026-10-15: v2c, v3 SHA-256/AES-128, GETBULK-Walk (`ifXTable` 54 Zeilen, `ifTable` 66 Zeilen),
falsches Passwort und SET-Ablehnung gegen den lokalen `snmpd` mit den Kommandozeilentools verifiziert.
Der Rust-Spike (`snmp2`) ist damit noch nicht getestet.

## Weitere Kombinationen testen

Zum Prüfen der auf EXOS/VOSS konfigurierten Kombination den Benutzer mit anderen Protokollen anlegen
(zum Beispiel `SHA` + `AES`, `SHA-512` + `AES`, `MD5` + `DES`). Pro Benutzer eine eigene `createUser`-Zeile
und ein eigenes `rouser`. Welche Protokolle der lokale `snmpd` unterstützt, zeigt `snmpd --help`/`man snmpd.conf`.

## Benutzer wieder entfernen

```bash
sudo systemctl stop snmpd
sudo sed -i '/netmontest/d' /var/lib/snmp/snmpd.conf /etc/snmp/snmpd.conf
sudo systemctl start snmpd
```
