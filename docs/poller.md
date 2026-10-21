# Poller, Status und Ereignislog

## Ablauf (`src/poller.rs`)

Beim Serverstart läuft ein **Supervisor**, der alle 15 s die Geräteliste liest und für jedes Gerät eine Aufgabe (Task) startet.
Jede Geräte-Aufgabe lädt in **jedem Zyklus** Gerät, aktivierte Sensoren und SNMP-Profil neu aus der Datenbank. Änderungen (Intervall,
Adresse, Zugangsdaten, neue oder deaktivierte Sensoren) wirken daher ohne Neustart; wird das Gerät gelöscht, endet die Aufgabe.

- **Startzeitpunkte** sind je Gerät deterministisch über das Intervall verteilt (höchstens 30 s Versatz): kein Burst zur vollen Minute.
  Die ersten Messwerte erscheinen deshalb bis zu 30 s nach dem Start.
- **Gleichzeitigkeit:** globale `Semaphore` (`poll_concurrency`, Standard 16); ein langsames Gerät blockiert nie andere.
- **Takt:** fester Takt je Gerät; überschreitet ein Zyklus das Intervall, geht es sofort weiter statt Aufholläufe zu starten.
- **Sensoren eines Geräts** laufen nacheinander (je ein GET). Jede Sensorabfrage ist zusätzlich durch `Timeout × (Wiederholungen + 1) + 2 s` begrenzt.
- **Geräteweiter SNMP-Fehler** (Timeout, Anmeldung fehlgeschlagen, nicht erreichbar): die übrigen SNMP-Sensoren desselben Zyklus werden nicht mehr
  versucht, sondern mit demselben Fehler markiert. Ping-Sensoren laufen trotzdem.
- **SNMP-Sitzung** wird je Zyklus neu aufgebaut (eine Runde Engine-Discovery bei v3). Das vermeidet veraltete Engine-Zeiten nach einem Gerätereboot.
- **Zähler-Basiswerte** bleiben über Zyklen erhalten, solange sich die Sensorkonfiguration nicht ändert. Nach einer Änderung kostet das einen Messwert.
- **Entschlüsselung** der Zugangsdaten nur im Poller (`storage::credentials::load_credentials`); Fehler dabei werden ohne Geheimnis protokolliert.

Config: `poller_enabled` (Standard `true`) und `poll_concurrency` (1–256, Standard 16). Beim Beenden stoppt der Server zuerst den Webserver
(Graceful Shutdown), dann den Poller (höchstens 10 s Wartezeit).

## Status (`src/status.rs`)

Sensorstatus: `Up`, `Warning`, `Down`, `Unknown`, `Paused`.

| Ergebnis der Abfrage | Status |
|---|---|
| Messung gelungen, Wert in Ordnung | `Up` |
| Messung gelungen, aber Interface nicht `up` (`ifOperStatus` ≠ 1) | `Down` (sofort) |
| Messung fehlgeschlagen, 1. Fehler in Folge | `Warning` |
| Messung fehlgeschlagen, ab dem 2. Fehler in Folge (`DOWN_AFTER_FAILURES`) | `Down` |
| nach einem Fehler wieder eine gelungene Messung | `Up` (sofort) |

Ein einzelnes verlorenes UDP-Paket löst so keinen Down-Alarm aus. Schwellwerte je Kanal (Warning/Down) kommen mit MVP-Schritt 4 in `assess`.
Gerätestatus = schlechtester Status der aktiven Sensoren (`Down` > `Warning` > `Unknown` > `Up`); deaktivierte Sensoren (`Paused`) zählen nicht,
nur deaktivierte ergeben `Paused`, keine Sensoren oder noch keine Abfrage ergeben `Unknown`.

## Datenbank (Migration `0006`)

- `sensor_status`: aktueller Zustand je Sensor (Status, seit wann, letzte Abfrage, Meldung auf Deutsch, Fehler in Folge).
- `events`: Protokoll der Statuswechsel (Zeit, Sensor, von → nach, Meldung); der erste Wert eines Sensors wird mit `from = NULL` protokolliert.
  Es bleiben nur die **neuesten 500 Statuswechsel** über alle Sensoren (`storage::status::EVENTS_KEPT`): `record_poll` löscht ältere in derselben
  Transaktion. Damit reichen auch die „letzten Statuswechsel“ der Sensor-Detailseite nur so weit zurück; das Statusband der Sensorliste nimmt für den
  Anfang der Stunde notfalls das „von“ des ältesten gespeicherten Wechsels oder den aktuellen Zustand (`status_history`).

## Protokoll (`/log`)

Menüpunkt **„Protokoll“** unter Monitoring, für alle Benutzer: Warnungen und Ausfälle sowie ihr Ende (Wechsel von Warnung/Ausfall nach „In Ordnung“)
über alle Sensoren, **neueste zuerst** (`web::log`, `storage::status::problem_log`). Spalten: Zeitpunkt, Status mit dem Status davor, Sensor, Gerät mit
Gruppenpfad, Meldung, Dauer. Die Dauer ist bei Warnung/Ausfall die Zeit bis zum nächsten Wechsel bzw. „dauert an“ mit der bisherigen Zeit, bei einer
Erholung „nach …“ die Dauer des Problems davor (aus `LAG`/`LEAD` über die Wechsel desselben Sensors). Erstabfragen, Pausieren und „Unbekannt“ stehen
nicht im Protokoll, zählen aber zu den 500 gespeicherten Wechseln. Oben rechts „Aktualisieren“ mit „Automatisch“.
- `storage::status::record_poll` schreibt Messwerte, Zustand und Ereignis in **einer Transaktion** und liefert den `StatusChange` zurück
  (die spätere Benachrichtigung hängt sich dort an; ein Wechsel `NULL → Up` ist kein Alarmgrund).

## Tests

Unit-Tests für die Statuslogik und `storage::status`; Poller-Tests gegen den Fake-Agenten (Messwerte, Warning → Down → Up, Fail-fast, Rate nach
dem zweiten Zyklus, deaktivierte/gelöschte Sensoren, Start und Stopp). Manuell gegen die echte Hardware:
`cargo test --test omada_manual poller_cycle -- --ignored --nocapture` (temporäre Datenbank, Switch und AP, zwei Zyklen).
