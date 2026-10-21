# Geräte, Gruppen und Verbindungstest

## Übersicht (Startseite)

Die Startseite zeigt unter den Kacheln einen **Baum wie in PRTG**: Gruppen mit Kästchen ⊟/⊞ zum Auf- und Zuklappen, darunter zuerst die eigenen
Geräte der Gruppe, dann die Untergruppen, beliebig tief und mit gepunkteten Linien verbunden. Jeder Eintrag ist eine kompakte Zeile mit dem Symbol seines Gerätetyps (Tooltip: Typ); rechts steht in einer
festen Spalte die **Sensorplakette**: Sind alle aktiven Sensoren im selben Zustand, eine Plakette wie „✓ 9 Sensoren“ (grün in Ordnung, gelb Warnung,
rot Ausfall, grau unbekannt), sonst je Zustand eine Plakette mit der Anzahl, schlechtester zuerst (✕ Ausfall, ! Warnung, ? unbekannt, ✓ in Ordnung).
Pausierte Sensoren stehen als umrandete Plakette ‖ am Ende; der Tooltip nennt jeweils Anzahl und Zustand. Gruppen zeigen die Summe ihres ganzen
Teilbaums (auch zugeklappt) und die Zahl der Geräte. Bei Geräten führen Name und Plaketten zur Sensorseite. Der Zustand der Gruppen (auf/zu) wird im
Browser gemerkt (`localStorage`, ohne Speicher sind alle offen). „Keine Geräte in dieser Gruppe“ erscheint nur bei wirklich leeren Gruppen.
Reihenfolge wie unten beschrieben.
Geräte ohne Gruppe stehen zuerst unter „Ohne Gruppe" (ohne Gruppen: „Alle Geräte“). Leser sehen Name, Adresse und Intervall;
Administratoren sehen zusätzlich das SNMP-Profil sowie als Symbole „Testen" (nur mit SNMP) und „Bearbeiten", an Gruppen „Reihenfolge“ und
„Gruppe bearbeiten“ sowie an Geräten und Gruppen „Pausieren“ ‖ bzw. „Fortsetzen“ ▶ (siehe unten). Pausierte Zeilen sind abgeblendet und nennen
die Pause („Pausiert bis …“, „Pausiert mit der Gruppe „Netzwerk““, Grund). Auf schmalen Bildschirmen (unter 560 px) entfallen Adresse und Intervall.
Die Aktionsmeldungen („Gerät angelegt." usw.) kommen über `?msg=` und werden nur auf feste Texte abgebildet.

**Aktualisieren:** Übersicht, Statuslisten, Sensorseite eines Geräts und Sensor-Detailseite haben oben rechts den Knopf **„Aktualisieren“** (nur das Symbol ↻, Name als Tooltip; lädt die
Seite neu, eine `?msg=`-Meldung fällt dabei weg) und den Schalter **„Automatisch“**: Die Seite lädt sich dann alle 60 s neu, ein Countdown steht daneben.
Der Countdown wartet („pausiert“), solange ein Eingabefeld den Fokus hat oder ein Formular geändert wurde (z. B. angehakte Sensoren), und läuft nur,
solange der Tab sichtbar ist; nach längerer Abwesenheit lädt die Seite beim Zurückkehren sofort neu. Der Schalter ist standardmäßig an und wird je Browser
gemerkt (`localStorage`). Ohne JavaScript bleibt „Aktualisieren“ ein normaler Link auf die Seite. Umsetzung: `templates/refresh.html`, `static/refresh.js`.

**Pfad oben auf Geräte- und Sensorseiten:** Übersicht › Gruppe › Untergruppe › Gerät › Sensor, z. B. „Übersicht › Netzwerk › Router › Router ER605 › Ping“
(`web::group_trail`, Teilvorlage `templates/device_trail.html`). Jede Gruppe führt zur Übersicht mit Sprungmarke `/#group-<id>`; `static/app.js`
klappt die Gruppe und die Gruppen darüber auf, springt hin und hebt den Gruppenkopf hervor. Geräte ohne Gruppe zeigen nur Übersicht › Gerät.

## Pausieren (nur Administratoren)

Wie in PRTG lassen sich **Gruppen, Geräte und einzelne Sensoren pausieren** (`src/pause.rs`, `storage::pauses`, Seiten `web::pause`):

- **Formular** `/pause/<group|device|sensor>/<id>` (Symbol ‖ in der Übersicht, Knopf auf der Sensorseite eines Geräts und auf der Detailseite):
  Dauer **bis zum Fortsetzen**, 15 Minuten, 1 Stunde, 3 Stunden, 1 Tag oder 1 Woche, dazu ein optionaler **Grund** (höchstens 200 Zeichen, z. B.
  „Firmware-Update“). Nach dem Speichern geht es zurück zur aufrufenden Seite (`back=overview|device|sensor`).
- **Vererbung:** Eine pausierte Gruppe pausiert alle Untergruppen, Geräte und Sensoren darunter, ein pausiertes Gerät alle seine Sensoren. Angezeigt
  wird die nächstliegende Pause: zuerst die eigene, dann die des Geräts, dann die der Gruppen von unten nach oben. Die eigenen Pausen darunter bleiben
  erhalten und gelten wieder, wenn die äußere endet.
- **Wirkung:** Pausierte Sensoren werden nicht abgefragt, lösen keine Mail aus und zählen nicht zum Gerätestatus; sie stehen als „Pausiert“ in Übersicht,
  Statuslisten (Kachel „Pausiert / unbekannt“) und auf der Sensorseite. Beim Pausieren wird ihr Zustand sofort auf „Pausiert“ gesetzt; ein laufender
  Ausfall endet damit im Protokoll, und beim Fortsetzen gibt es **keine** nachträgliche „Wieder in Ordnung“-Mail (die erste Abfrage danach ist ein Neustart
  ohne alten Zustand).
- **Fortsetzen** (`POST /resume/<kind>/<id>`) geht nur an dem Objekt, das selbst pausiert ist; ein Gerät, das nur mit seiner Gruppe pausiert ist,
  zeigt den Hinweis und wird über die Gruppe fortgesetzt. **Zeitlich begrenzte Pausen** enden von selbst: Der Poller prüft alle 15 s
  (`pauses::expire`); schon vorher gilt eine abgelaufene Pause nicht mehr.
- **Protokoll:** Pausieren (mit Benutzer, Grund und geplantem Ende), Fortsetzen und automatisches Ende stehen im Protokoll `/log`
  (Tabelle `pause_log`, die neuesten 500 Einträge, `pauses::LOG_KEPT`; siehe `docs/poller.md`).
- **Datenbank** (Migration `0016`): `groups`/`devices` haben `paused`, `pause_until_ms`, `pause_reason`; bei Sensoren bleibt die eigene Pause
  `enabled = 0` (dazu `pause_until_ms`, `pause_reason`). Sensoren, die vorher „deaktiviert“ waren, sind seitdem unbefristet pausiert.

## Reihenfolge (nur Administratoren)

- **Automatisch** (Standard): nach Namen, Groß-/Kleinschreibung egal, Zahlen der Größe nach („Switch 2“ vor „Switch 10“).
- **Von Hand**: „Reihenfolge“ an einer Gruppe ordnet deren Untergruppen und Geräte (`/groups/<id>/order`), am Bereich „Ohne Gruppe“
  bzw. „Alle Geräte“ die Geräte ohne Gruppe und die obersten Gruppen (`/groups/0/order`). Dieselbe Seite öffnet „Gruppen anordnen“ oben auf der
  Übersicht; der Knopf erscheint nur bei mindestens zwei obersten Gruppen. Zeilen am Griff ziehen, mit ↑/↓ verschieben oder
  eine Position eintragen und speichern. Ohne JavaScript bleiben die Positionsfelder.
- Von Hand geordnete Einträge stehen vorn, neue und nicht geordnete dahinter in automatischer Reihenfolge.
  „Automatisch sortieren“ verwirft die Handordnung der Ebene.
- Wird ein Gerät in eine andere Gruppe (oder eine Gruppe unter eine andere Gruppe) verschoben, verliert es seinen Platz und wird dort automatisch einsortiert.
  Umbenennen ändert den Platz nicht.
- Die Reihenfolge gilt für alle Benutzer. Technik: Spalte `position` in `groups`, `devices`, `sensors` (Migration `0010`, `NULL` = automatisch),
  Sortierung in `src/ordering.rs`, Seiten in `src/web/order.rs`, Skript `static/order.js`.

## Gruppen (nur Administratoren)

- Hierarchisch, höchstens **8 Ebenen**. Name 1–64 Zeichen, **pro Ebene eindeutig** (Groß-/Kleinschreibung egal, nur für ASCII-Buchstaben).
- Anlegen, Umbenennen und Verschieben (`/groups/…`). Eine Gruppe kann nicht in sich selbst oder in eine ihrer Untergruppen verschoben werden;
  im Formular werden diese Ziele gar nicht angeboten.
- Löschen nur, wenn die Gruppe **weder Untergruppen noch Geräte** enthält (Bestätigungshaken nötig).

## Geräte (nur Administratoren)

| Feld | Regel |
|---|---|
| Name | 1–64 Zeichen, eindeutig (Groß-/Kleinschreibung egal) |
| Gerätetyp | Allgemein (Standard), Router, Switch, Access Point, Firewall, Server, NAS / Speicher, PC, Drucker, Kamera, Smart Home / IoT, Internetdienst; bestimmt nur das Symbol in der Übersicht |
| Adresse | IPv4/IPv6 oder Hostname nach RFC 1123 (wird kleingeschrieben); `0.0.0.0`, `::`, Multicast und Broadcast sind nicht erlaubt; `localhost` ist erlaubt |
| Gruppe | optional |
| SNMP-Profil | ein Profil oder **„Kein SNMP – nur Ping“**; ein von Geräten verwendetes Profil lässt sich nicht löschen (Meldung statt Fehler) |
| UDP-Port | 1–65535, Standard 161 |
| Intervall | 10–3600 s, Standard 60 s |
| Timeout je Versuch | 1–60 s, Standard 5 s |
| Wiederholungen | 0–5, Standard 1 |

Zusätzlich muss **Timeout × (Wiederholungen + 1) ≤ Intervall** gelten, damit sich Abfragen eines Geräts nie überlappen.
Alle Prüfungen laufen serverseitig; bei einem Fehler bleiben die eingegebenen Werte im Formular erhalten (Passwörter gibt es hier nicht).

„Gerät bearbeiten“ zeigt in der Pfadleiste die Gruppen und das Gerät (Link auf seine Sensorseite). „Abbrechen“ und „Speichern“ führen zurück zur Seite,
von der aus das Formular geöffnet wurde: zur Übersicht (Link dort mit `?back=overview`) oder sonst zur Sensorseite des Geräts.

## Geräte ohne SNMP (nur Ping)

Für Geräte, die kein SNMP sprechen (z. B. der ER605, Drucker, Kameras), im Feld SNMP-Profil **„Kein SNMP – nur Ping“** wählen. Das geht auch,
bevor es überhaupt ein SNMP-Profil gibt.

- Beim Anlegen kommt gleich ein aktiver Sensor **„Ping“** dazu; die Seite springt auf die Sensorseite des Geräts. Eine gewählte Gerätevorlage gilt dann nicht.
- Auf der Sensorseite fehlen „Verbindung testen“, „Werte suchen“, „Interfaces suchen“ und der Bereich Gerätevorlage; „Sensor hinzufügen“ bietet nur Ping.
  Die Übersicht zeigt „kein SNMP“ statt des Profils und keinen Knopf „Testen“. Die SNMP-Aktionen werden auch serverseitig mit einer Erklärung abgelehnt.
- Später lässt sich unter „Gerät bearbeiten“ ein SNMP-Profil wählen. Zurück zu „Kein SNMP“ geht nur, wenn das Gerät keine SNMP-Sensoren mehr hat.
- Technik: `devices.profile_id` ist `NULL`. Der Poller baut für Geräte ohne SNMP-Sensoren keine SNMP-Sitzung auf; hätte ein solches Gerät doch einen
  SNMP-Sensor, meldet dieser „Für dieses Gerät ist kein SNMP eingerichtet (nur Ping).“
- Die Spalte war bis Version 0011 Pflicht. SQLite lässt das nur durch einen Neuaufbau der Tabelle ändern, der abgeschaltete Fremdschlüssel braucht
  (sonst würde das Löschen der alten Tabelle alle Sensoren mitnehmen) – das geht nicht in einer sqlx-Migration. Deshalb erledigt
  `storage::migrate` das einmalig beim Start (`allow_devices_without_profile`: neue Tabelle, Daten kopieren, Indizes, `PRAGMA foreign_key_check`,
  erst dann `COMMIT`); danach ist es ein No-op. Vorher eine Sicherung mit `opennetmon backup` machen.

## Verbindungstest

Am Gerät (Übersicht „Testen" oder Bearbeitungsseite „Verbindung testen"). Der Server fragt `sysDescr.0` und `sysUpTime.0` mit dem
gespeicherten Profil ab und zeigt Gerätebeschreibung, Laufzeit und Antwortzeit oder eine verständliche Fehlermeldung:

| Meldung | Ursache |
|---|---|
| „Keine Antwort innerhalb des Timeouts …" | Gerät oder Port nicht erreichbar, SNMP aus, falsche Community oder falsche v3-Benutzerdaten (bei v2c antworten Geräte auf eine falsche Community nicht) |
| „Anmeldung fehlgeschlagen …" | SNMPv3: Passwort, Auth- oder Verschlüsselungsverfahren stimmen nicht |
| „… nicht erreichbar oder der SNMP-Port ist geschlossen" | Hostname nicht auflösbar oder ICMP „Port unreachable" |

Der Test verwendet höchstens **10 s Timeout und 1 Wiederholung**, egal wie das Gerät konfiguriert ist (die Anfrage blockiert den Browser nur kurz).
Er ist POST-only mit CSRF-Token und nur für Administratoren, da der Server damit SNMP-Pakete an frei wählbare Adressen schickt.
Passwörter erscheinen weder auf der Ergebnisseite noch im Log (das Log nennt nur Benutzer, Gerät und ok/fehlgeschlagen).

## Technik

- Tabellen `groups` und `devices` (Migration `0004`), Fremdschlüssel mit `ON DELETE RESTRICT` für Gruppe und Profil.
- Gerätetyp: Spalte `devices.kind` (Migration `0015`, Text ohne `CHECK`, damit neue Typen keinen Tabellenneubau brauchen; unbekannte Werte gelten als
  „Allgemein“), im Code `devices::DeviceKind`; die Symbole stehen im Makro `device_icon` in `templates/home.html`. Formulare ohne das Feld ergeben „Allgemein“.
- SNMP-Client `src/snmp/mod.rs`: `SnmpClient::connect` (inkl. SNMPv3-Engine-Discovery), `get` mit Timeout, Wiederholungen bei Zeitüberschreitung und
  einmaliger Wiederholung nach einem v3-Kontext-Update. Der Poller nutzt später denselben Client.
- `snmp2` mit Feature `heap_buffers`: ohne dieses Feature liegt ein 64-KB-Puffer auf dem Stack, was in Tokio-Threads (2 MB Stack) im Debug-Build überläuft.

## Tests

- Unit- und Handler-Tests nutzen einen kleinen **Fake-SNMPv2c-Agenten** (`src/snmp/testagent.rs`), der wie echte Agenten bei falscher Community schweigt.
- `tests/snmp_local.rs`: Integrationstests gegen den lokalen `snmpd` (v2c, v3 mit `OPENNETMON_TEST_V3_*`); überspringen sich selbst, wenn der Agent oder die Variablen fehlen.
- `tests/omada_manual.rs`: manuelle Tests gegen die echten Omada-Geräte, alle `#[ignore]`:
  `set -a; . ~/.config/opennetmon-dev/omada.env; set +a; cargo test --test omada_manual -- --ignored --nocapture`
