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
| Dieser Server (`local`) | je nach Wert, siehe „Dieser Server“ | liest den Rechner, auf dem opennetmon läuft, direkt (kein SNMP) |

Die Traffic-Raten sind Bit/s (Oktette × 8). Eine Abfrage braucht je Sensor genau ein SNMP-GET; Zähler holen zusätzlich `sysUpTime`.

## Dieser Server (`src/sensors/local.rs`, Seite `web::server_values`)

Sensoren für den Rechner, auf dem opennetmon selbst läuft – ohne `snmpd`, ohne Zugangsdaten, nur lesend. Was gemessen wird, steht in der Spalte
`sensors.target`:

| Wert (`target`) | Kanäle | Quelle | Grenzen beim Anlegen |
|---|---|---|---|
| CPU-Last (`cpu`) | `total`, `iowait` (%) | `/proc/stat`, Differenz zweier Abfragen (die erste liefert noch keinen Wert) | keine (kurze Spitzen sind normal) |
| Load (`load`) | `load1`, `load5`, `load15` | `/proc/loadavg` | keine |
| Arbeitsspeicher (`memory`) | `used` (%, ohne Puffer/Cache), `available` (Bytes), `swap_used` (%; ohne Swap 0) | `/proc/meminfo` (`MemAvailable`) | `used`: Warnung über 90 %, Ausfall über 95 % |
| Laufzeit (`uptime`) | `uptime` (s) | `/proc/uptime` | keine |
| Dateisystem (`disk:<Einhängepunkt>`) | `used` (%), `free` (Bytes) | `statvfs` (wie `df`: reservierte Blöcke zählen als belegt), höchstens 5 s | `used`: Warnung über 85 %, Ausfall über 95 % |
| Temperatur (`temp:<Chip>/<Bezeichnung>`) | `temperature` (°C) | `/sys/class/hwmon/*/temp*_input` | Warnung über `temp*_max`, Ausfall über `temp*_crit` (wenn die Hardware sie meldet) |
| Netzwerk-Traffic (`net:<Interface>`) | `in`, `out` (bit/s) | `/sys/class/net/<Interface>/statistics/rx_bytes`/`tx_bytes` (64 Bit) | keine |

- **Anlegen:** Auf der Sensorseite jedes Geräts unter „Sensor hinzufügen“ den Typ „Dieser Server (CPU, Speicher, Dateisysteme …)“ wählen, oder oben
  der Knopf „Werte dieses Servers“ (nur für Administratoren). Der Knopf erscheint bei Geräten vom Typ **Server**, bei Geräten mit der Adresse
  `127.x.x.x`, `::1` oder `localhost` und bei Geräten, die schon solche Sensoren haben. Die Seite listet, was der Rechner gerade
  meldet, mit aktuellem Wert; Allgemeines, lokale Dateisysteme und aktive Netzwerkkarten sind vorausgewählt, Temperaturen und Netzlaufwerke nicht.
  Vorhandene Werte sind abgehakt und gesperrt; angelegt wird nur, was der Rechner in dem Moment anbietet.
- **Empfehlung:** Den Server als eigenes Gerät anlegen (Typ „Server“, ohne SNMP, mit seiner LAN-Adresse für den Ping). Die Werte stammen aber immer vom Rechner,
  auf dem opennetmon läuft – auch wenn sie an einem anderen Gerät angelegt werden.
- **Dateisysteme:** Nur echte Dateisysteme (ext4, xfs, btrfs, zfs, vfat, NFS, CIFS …); Pseudo-Dateisysteme und Snap-Images (squashfs) fehlen. Jedes Gerät
  erscheint einmal (Bind-Mounts wie systemds schreibgeschützte Sicht auf `/` zählen nicht). Unter der Härtung der Unit ist `/home` nicht sichtbar
  (`ProtectHome=yes`); liegt `/home` auf einem eigenen Dateisystem, fehlt es in der Liste.
- **Temperaturen:** Die hwmon-Nummern ändern sich zwischen Neustarts, darum besteht der Schlüssel aus Chipname und Bezeichnung (`coretemp/Package id 0`);
  ein zweiter Chip gleichen Namens heißt `nvme#2`. Fehlt der Sensor nach einem Hardwaretausch, meldet der Sensor „Unbrauchbarer Messwert“.
- **Netzwerk:** alle Interfaces außer `lo`; „Netzwerkkarte“ heißt, dass ein Gerät dahintersteht (`/sys/class/net/<Interface>/device`), sonst „Virtuell“
  (Bridge, VPN, Container). Ein verschwundenes Interface meldet einen Fehler.
- **Unit:** Die Sensoren brauchen die Systemdateien in `/proc`; `deploy/opennetmon.service` setzt deshalb kein `ProcSubset=pid` mehr
  (`systemd-analyze security` bleibt bei 1,3). Alles andere an der Härtung bleibt.
- **Vorlagen:** Gerätevorlagen übernehmen diese Sensoren nicht, sie gehören zu genau einem Rechner.
- **Datenbank:** Neuer Typ `local` und Spalte `target`. Der `CHECK` der Tabelle lässt sich in SQLite nur durch Neuaufbau ändern; das macht
  `storage::migrate` einmalig beim ersten Start mit abgeschalteten Fremdschlüsseln (`allow_local_sensors`, wie bei den Geräten ohne SNMP).

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
- **Statuslisten** (`/status/<filter>`, `src/web/status_list.rs`, alle angemeldeten Benutzer): Jede Kachel der Übersicht ist ein Link auf die Liste
  ihrer Sensoren über alle Geräte – `all` (Sensoren), `up` (In Ordnung), `warning`, `down` und `inactive` (Pausiert / unbekannt). Oben stehen dieselben
  Kacheln wie auf der Übersicht (gemeinsames Template `templates/status_tiles.html`, `status_list::tiles`); die gezeigte Liste hat einen blauen Ring. Spalten: Status, Sensor, Gerät mit Gruppenpfad, Meldung, aktueller Wert (wie auf der Sensorseite) und
  „Seit“ (Zeitpunkt des letzten Statuswechsels und Dauer; leer bei pausierten und noch nie abgefragten Sensoren; bei pausierten steht in der Meldung die Pause).
  Sortiert wird per Klick auf die Spaltenköpfe (`?sort=status|sensor|device|value|since&dir=asc|desc`, ohne Skript, erneuter Klick dreht um).
  Standard: Warnung und Ausfall neueste Wechsel zuerst, „Sensoren“ schlechtester Status zuerst, sonst nach Gerät. Beim Wert bleiben Sensoren gleicher Einheit
  zusammen (Traffic zuerst, dann Raten, ms, %, Anzahl, Laufzeit, Statuswerte), sortiert wird innerhalb der Einheit nach der Summe der Kanäle
  (Traffic: ein + aus). Zeilen ohne Wert oder Zeitpunkt stehen immer am Ende; Gleichstände bleiben je Gerät in der Sensorreihenfolge.
- **Sensorseite** (`/devices/<id>/sensors`, alle angemeldeten Benutzer): Tabelle mit Name, Typ, Status, letzter Meldung, aktuellem Wert
  (je Kanal eine Zeile ohne Umbruch, z. B. Traffic ein/aus untereinander, Kanalnamen als Tooltip; nur wenn der letzte Wert höchstens drei Intervalle plus eine Minute alt ist) und einem
  **Mini-Graphen der letzten Stunde** (`src/web/sparkline.rs`, als SVG vom Server, ohne Skript; Klick führt zur Detailseite):
  - Messwerte (Ping, Traffic, eigene OID): eine Linie je Kanal in den Farben der großen Graphen, gemeinsame Skala ab 0, Mittelwerte in
    Abschnitten von mindestens 1 Minute und mindestens zwei Abfrageintervallen (bei 60 s also 30 Abschnitte zu 2 Minuten;
    `storage::measurements::bucket_averages`);
    Abschnitte ohne Wert unterbrechen die Linie.
  - Statuswerte, Interface-Status und Uptime: ein Statusband (grün/gelb/rot) aus dem Ereignislog (`storage::status::status_history`),
    weil eine Kurve dort nichts aussagt.
  - Pausierte Sensoren haben keinen Graphen; auf schmalen Bildschirmen (unter 560 px) ist die Spalte ausgeblendet.
  Die Zeit der letzten Abfrage steht auf der Detailseite.
  Administratoren sehen zusätzlich je Sensor „Pausieren“ und „Löschen“, oben „Gerät pausieren“ bzw. „Fortsetzen“ (`docs/geraete-und-gruppen.md`, „Pausieren“), „Interfaces suchen“, „Werte suchen“ (`docs/mibs.md`) und das Formular „Sensor hinzufügen“ (Ping, Uptime, eigene OID mit
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
- **Pausierte Sensoren** (selbst pausiert: `enabled = false`, Status „Pausiert“) werden nicht abgefragt und stehen in einem eigenen aufklappbaren Bereich
  „Pausierte Sensoren (n)“ mit der Pause in der Spalte „Meldung“ und Sammelaktionen (fortsetzen, alle fortsetzen, löschen; `POST /devices/<id>/sensors/bulk`,
  nur Sensoren dieses Geräts). Ist das ganze Gerät oder seine Gruppe pausiert, steht darüber ein Hinweis und alle Sensoren gelten als pausiert.
- **Anlegen** (`POST /devices/<id>/sensors/interfaces`): Felder `t_<ifIndex>`, `s_<ifIndex>` und `n_<ifIndex>` (Anzeigename). Sensornamen „Traffic <Name>“ bzw.
  „Status <Name>“ (höchstens 64 Zeichen); bei gleichem Namen wird „ #<Index>“ angehängt. Bereits vorhandene Interface-Sensoren werden nie doppelt angelegt.
- Gemessen an der echten Hardware: Switch (9 Interfaces, Indizes 1 und 49153–49160, `ifName` z. B. „gigabitEthernet 1/0/1“, Alias „Port1“) und AP (13 Interfaces,
  nur `ifDescr`, z. B. `eth0`, `ath0`).
