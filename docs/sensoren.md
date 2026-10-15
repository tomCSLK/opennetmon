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
