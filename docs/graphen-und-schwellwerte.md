# Detailseite, Graphen, Schwellwerte, Rollups

## Detailseite (`/sensors/<id>`, alle angemeldeten Benutzer)

Kopf mit Typ, Gerät, Status, Meldung und letzter Abfrage; Zeitraum-Schalter **1 Stunde / 24 Stunden / 7 Tage / 30 Tage** (`?range=1h|24h|7d|30d`,
ungültige Werte ergeben 24 Stunden); Graph; Tabelle mit dem letzten Wert je Kanal; Schwellwerte; die letzten 20 Statuswechsel dieses Sensors.
Die Sensorliste eines Geräts (`/devices/<id>/sensors`) verlinkt jeden Sensor dorthin.

## Graph (uPlot)

- **uPlot 1.6.32** (MIT) liegt unverändert unter `static/vendor/uplot/` (mit Lizenz, Quelle und Prüfsumme in der dortigen `README.md`) und wird mit dem Binary
  ausgeliefert – kein CDN, kein npm. Eigener Code: `static/chart.js` (kein Inline-Skript, die CSP bleibt `default-src 'self'`).
- Daten kommen als JSON von `/sensors/<id>/data?range=…`: `{"range","ts":[Sekunden…],"series":[{"key","label","unit","values":[…|null]}]}`.
  Alle Kanäle teilen die Zeitachse; fehlt ein Wert, ist er `null` (Lücke im Graph, keine Linie).
- Je Einheit ein Diagramm (Eingehend und Ausgehend teilen sich eines). Anzeige in der **Zeitzone des Browsers**, 24-Stunden-Uhr, deutsche Zahlen und Einheiten
  (bit/s, kbit/s, Mbit/s, Gbit/s). Der Graph lädt alle 30 s nach.
- Herkunft der Punkte: 1 h und 24 h Rohwerte, 7 d Mittelwerte in 10-Minuten-Blöcken, 30 d Stundenwerte aus `samples_1h` (solange noch keine Rollups
  existieren, Stundenmittel direkt aus den Rohdaten). Höchstens 20 000 Punkte je Kanal.
- Statische Dateien haben `Cache-Control: max-age=3600`; nach einem Update kann der Browser bis zu eine Stunde das alte `chart.js` zeigen (Strg+F5).

## Schwellwerte

Je Kanal (nicht bei Statuskanälen) vier optionale Grenzen: **Warnung über / unter** und **Down über / unter**. Eingabe in der Anzeigeeinheit
(Verkehr in Mbit/s, Dezimalkomma erlaubt), gespeichert in der Basiseinheit (`channels.warn_above` usw., Migration `0007`).

- Ein Wert löst aus, wenn er die Grenze **strikt über- bzw. unterschreitet**; Down hat Vorrang vor Warnung.
- Warnung → Sensorstatus `Warning`, Down-Grenze → `Down`, jeweils **sofort ohne Entprellung** (anders als Messfehler, siehe `docs/poller.md`);
  die Meldung nennt Kanal, Wert und Grenze („Eingehend: 90 Mbit/s liegt über der Down-Grenze 80 Mbit/s“).
- Prüfung: Down-Grenzen dürfen nicht milder als Warngrenzen sein, die obere Grenze nicht unter der unteren liegen, alles endlich. Fehler erscheinen auf der
  Seite, die Eingaben bleiben stehen. Nur Administratoren können speichern (CSRF-geschützt).
- Wirksam ab dem nächsten Zyklus (der Poller liest die Grenzen jedes Mal neu).

## Rollups und Aufbewahrung (`src/maintenance.rs`)

Der Poller startet zusätzlich eine Wartungsaufgabe (30 s nach dem Start, danach alle 10 Minuten):

1. **Rollup:** `samples_1h` (Stunde, min/avg/max/Anzahl) aus den Rohwerten, `samples_1d` (Tag, gewichteter Mittelwert) aus den Stundenwerten. Es werden die letzten
   2 Stunden bzw. 1 Tag vor dem neuesten Rollup-Eintrag neu berechnet (unfertige Stunde, nachgelieferte Werte); wiederholtes Ausführen ändert nichts.
2. **Aufbewahrung:** Rohwerte `retention_raw_days` (Standard 7), Stundenwerte und Ereignisse `retention_hourly_days` (Standard 365); **Tageswerte bleiben unbegrenzt**.
   Aggregiert wird immer vor dem Löschen.

Config: `retention_raw_days` (1–3650), `retention_hourly_days` (mindestens `retention_raw_days`, höchstens 36500). Läuft der Server länger als die Rohdaten-Aufbewahrung
nicht, fehlen die Stundenwerte dieser Zeit. Ohne laufenden Poller (`poller_enabled = false`) gibt es auch keine Wartung.

## Tests

Statuslogik und Schwellwerte (`src/status.rs`), Formatierung (`src/sensors/format.rs`), Rollups und Aufbewahrung (`src/maintenance.rs`), Zeitreihen (`src/storage/measurements.rs`),
Handler (`src/web/sensor_detail.rs`: Anzeige, JSON, Speichern, Rechte, CSRF, Assets), Poller (Schwellwerte ändern den Status). Der Graph selbst wurde im Browser
gegen den echten Switch geprüft (Darstellung, keine Konsolenfehler/CSP-Verstöße).
