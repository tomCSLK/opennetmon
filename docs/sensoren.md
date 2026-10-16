# Sensoren und Messdaten

Ein **Sensor** gehört zu einem Gerät und liefert bei jeder Abfrage einen oder mehrere **Kanäle** (Messwerte).
Alle Sensortypen implementieren das Trait `Sensor` (`src/sensors/mod.rs`); der Poller kennt keine Sondertypen.

| Typ (`kind`) | Kanäle | Abfrage |
|---|---|---|
| Ping (`ping`) | `rtt` (ms) | ICMP-Echo, Timeout wie am Gerät eingestellt |
| Uptime (`uptime`) | `uptime` (s) | `sysUpTime.0` |
| Interface-Traffic (`if_traffic`) | `in`, `out` (bit/s) | `ifHCInOctets`/`ifHCOutOctets` (64 Bit), sonst `ifInOctets`/`ifOutOctets` (32 Bit) |
| Interface-Status (`if_status`) | `oper_status` (Code) | `ifOperStatus` (1 = up, 2 = down, 3 = testing, 5 = dormant, 7 = lowerLayerDown) |
| OID (`oid`) | `value` | beliebige numerische OID, Modus *Gauge* (Wert × Faktor) oder *Counter* (Rate/s × Faktor) |

Die Traffic-Raten sind Bit/s (Oktette × 8). Eine Abfrage braucht je Sensor genau ein SNMP-GET; Zähler holen zusätzlich `sysUpTime`.

## Zähler und Raten (`src/sensors/counter.rs`)

Rate = Differenz / Zeitdelta. **Es wird kein Wert geschrieben** (statt eines Ausreißers), wenn

- es der erste Wert nach dem Start oder nach einem Reset ist (ein Reset kostet genau einen Wert),
- `sysUpTime` kleiner geworden ist (Gerät neu gestartet),
- ein 64-Bit-Zähler gesunken ist,
- ein 32-Bit-Zähler gesunken ist und weder `sysUpTime` noch ein plausibles Raten-Limit einen echten Überlauf bestätigen,
- die Zeit nicht vorangeschritten ist.

Ein bestätigter 32-Bit-Überlauf (Access Points) wird korrekt gerechnet. Wechselt ein Gerät zwischen 64- und 32-Bit-Zählern, beginnt der Sensor neu.

## Datenbank (Migration `0005`)

- `sensors`: Gerät (`ON DELETE CASCADE`), Name (je Gerät eindeutig, Groß-/Kleinschreibung egal), Typ und die zum Typ passenden Spalten
  (`if_index`; `oid`, `oid_mode`, `scale`); ein `CHECK` erzwingt, dass genau die passenden Spalten gesetzt sind; `enabled`.
- `channels`: je Sensor die Kanäle des Typs (`key`, `unit`), werden beim Anlegen des Sensors in derselben Transaktion erzeugt.
- `samples`: `(channel_id, ts, value)`, `ts` in Millisekunden seit Unix-Epoche (UTC), Primärschlüssel `(channel_id, ts)`
  (`WITHOUT ROWID`); ein zweiter Wert zum selben Zeitstempel wird ignoriert. Sensor löschen entfernt Kanäle und Messwerte mit.
- Zugriff: `storage::sensors` (`create_sensor`, `list_sensors`, `list_enabled_sensors`, `set_enabled`, `delete_sensor`, `insert_samples`, `samples_since`).

## Eingaben (`src/sensors/spec.rs`)

OIDs nur numerisch (`1.3.6.1.2.1.1.3.0`, führender Punkt erlaubt, mindestens 2 Stellen, höchstens 128), Interface-Index 1 bis 2147483647,
Faktor ungleich 0 und endlich. Der Index ist bei den Switches nicht fortlaufend (z. B. 49153).

## Ping

`surge-ping` mit unprivilegierten ICMP-Sockets (`net.ipv4.ping_group_range`); ohne Berechtigung bleibt die Adressfamilie unbenutzbar und der Ping
schlägt mit klarer Meldung fehl. Auf dem Server: `ping_group_range` setzen oder `AmbientCapabilities=CAP_NET_RAW` in der systemd-Unit.

## Tests

Unit-Tests gegen den Fake-Agenten (`src/snmp/testagent.rs`, jetzt mit setzbarer OID-Tabelle), Storage-Tests mit temporärer Datenbank.
Manuell gegen die echte Hardware: `cargo test --test omada_manual sensors_work -- --ignored --nocapture` (Switch 64 Bit, AP 32 Bit, Ping).

## Oberfläche (`src/web/sensors.rs`)

- **Übersicht** (`/`): je Gerät Status (schlechtester Sensorstatus), Link auf die Sensorseite und Anzahl der Sensoren.
- **Sensorseite** (`/devices/<id>/sensors`, alle angemeldeten Benutzer): Tabelle mit Name, Typ, Status, letzter Meldung und letzter Abfrage (UTC).
  Administratoren sehen zusätzlich „Deaktivieren“, „Löschen“, „Interfaces suchen“, „Werte suchen“ (`docs/mibs.md`) und das Formular „Sensor hinzufügen“ (Ping, Uptime, eigene OID mit
  Modus Wert/Zähler und Faktor; Dezimalkomma wird akzeptiert). Fehler erscheinen auf derselben Seite, Eingaben bleiben erhalten.
- **Reihenfolge**: automatisch nach Typ (Ping, Uptime, Traffic, Interface-Status, dann eigene Werte), innerhalb eines Typs nach Namen
  mit Zahlen der Größe nach (Port 2 vor Port 10). „Reihenfolge ändern“ (`/devices/<id>/sensors/order`, nur Administratoren) ordnet von Hand
  per Ziehen, ↑/↓ oder Positionsnummer; Regeln wie bei Geräten (`docs/geraete-und-gruppen.md`).
- **Umbenennen** auf der Detailseite (Bereich „Name“, nur Administratoren, `POST /sensors/<id>/rename`): höchstens 64 Zeichen, je Gerät eindeutig;
  Messwerte, Graph, Schwellwerte, Status und Ereignisse bleiben erhalten.
- **Interface-Discovery** (`POST /devices/<id>/discover`, nur Administratoren, CSRF): `SnmpClient::walk` (GETBULK) liest `ifDescr`, `ifType`, `ifSpeed`,
  `ifOperStatus`, `ifName`, `ifHighSpeed` und `ifAlias` (`src/discovery.rs`) und führt sie je `ifIndex` zusammen. Name = `ifName`, sonst `ifDescr`
  (Access Points haben keine `ifXTable`), sonst „Interface N“. Die Seite zeigt je Interface zwei Häkchen (Traffic, Status). **Vorausgewählt** ist Traffic bei
  aktiven Ports der Typen Ethernet/WLAN; vorhandene Sensoren sind gesperrt. Der Test begrenzt wie der Verbindungstest auf 10 s Timeout und 1 Wiederholung,
  höchstens 2000 Zeilen je Tabelle.
- **Inaktive Sensoren** (`enabled = false`, Status „Inaktiv“) werden nicht abgefragt und stehen in einem eigenen aufklappbaren Bereich
  mit Sammelaktionen (aktivieren, alle aktivieren, löschen; `POST /devices/<id>/sensors/bulk`, nur Sensoren dieses Geräts).
- **Anlegen** (`POST /devices/<id>/sensors/interfaces`): Felder `t_<ifIndex>`, `s_<ifIndex>` und `n_<ifIndex>` (Anzeigename). Sensornamen „Traffic <Name>“ bzw.
  „Status <Name>“ (höchstens 64 Zeichen); bei gleichem Namen wird „ #<Index>“ angehängt. Bereits vorhandene Interface-Sensoren werden nie doppelt angelegt.
- Gemessen an der echten Hardware: Switch (9 Interfaces, Indizes 1 und 49153–49160, `ifName` z. B. „gigabitEthernet 1/0/1“, Alias „Port1“) und AP (13 Interfaces,
  nur `ifDescr`, z. B. `eth0`, `ath0`).
