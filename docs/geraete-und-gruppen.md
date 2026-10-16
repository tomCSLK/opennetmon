# Geräte, Gruppen und Verbindungstest

## Übersicht (Startseite)

Die Startseite zeigt alle Gruppen als Baum (Untergruppen eingerückt) mit den zugehörigen Geräten, in der Reihenfolge unten.
Geräte ohne Gruppe stehen zuerst unter „Ohne Gruppe". Leser sehen Name, Adresse und Intervall;
Administratoren sehen zusätzlich das SNMP-Profil sowie die Aktionen „Testen" und „Bearbeiten".
Die Aktionsmeldungen („Gerät angelegt." usw.) kommen über `?msg=` und werden nur auf feste Texte abgebildet.

## Reihenfolge (nur Administratoren)

- **Automatisch** (Standard): nach Namen, Groß-/Kleinschreibung egal, Zahlen der Größe nach („Switch 2“ vor „Switch 10“).
- **Von Hand**: „Reihenfolge ändern“ oben auf der Übersicht ordnet die obersten Gruppen und die Geräte ohne Gruppe (`/groups/0/order`),
  „Reihenfolge“ an einer Gruppe deren Untergruppen und Geräte (`/groups/<id>/order`). Zeilen am Griff ziehen, mit ↑/↓ verschieben oder
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
| Adresse | IPv4/IPv6 oder Hostname nach RFC 1123 (wird kleingeschrieben); `0.0.0.0`, `::`, Multicast und Broadcast sind nicht erlaubt; `localhost` ist erlaubt |
| Gruppe | optional |
| SNMP-Profil | Pflicht; ein von Geräten verwendetes Profil lässt sich nicht löschen (Meldung statt Fehler) |
| UDP-Port | 1–65535, Standard 161 |
| Intervall | 10–3600 s, Standard 60 s |
| Timeout je Versuch | 1–60 s, Standard 5 s |
| Wiederholungen | 0–5, Standard 1 |

Zusätzlich muss **Timeout × (Wiederholungen + 1) ≤ Intervall** gelten, damit sich Abfragen eines Geräts nie überlappen.
Alle Prüfungen laufen serverseitig; bei einem Fehler bleiben die eingegebenen Werte im Formular erhalten (Passwörter gibt es hier nicht).

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
- SNMP-Client `src/snmp/mod.rs`: `SnmpClient::connect` (inkl. SNMPv3-Engine-Discovery), `get` mit Timeout, Wiederholungen bei Zeitüberschreitung und
  einmaliger Wiederholung nach einem v3-Kontext-Update. Der Poller nutzt später denselben Client.
- `snmp2` mit Feature `heap_buffers`: ohne dieses Feature liegt ein 64-KB-Puffer auf dem Stack, was in Tokio-Threads (2 MB Stack) im Debug-Build überläuft.

## Tests

- Unit- und Handler-Tests nutzen einen kleinen **Fake-SNMPv2c-Agenten** (`src/snmp/testagent.rs`), der wie echte Agenten bei falscher Community schweigt.
- `tests/snmp_local.rs`: Integrationstests gegen den lokalen `snmpd` (v2c, v3 mit `NETMON_TEST_V3_*`); überspringen sich selbst, wenn der Agent oder die Variablen fehlen.
- `tests/omada_manual.rs`: manuelle Tests gegen die echten Omada-Geräte, alle `#[ignore]`:
  `set -a; . ~/.config/netmon-dev/omada.env; set +a; cargo test --test omada_manual -- --ignored --nocapture`
