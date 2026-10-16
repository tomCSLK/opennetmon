# Detailseite, Graphen, Schwellwerte, Rollups

## Detailseite (`/sensors/<id>`, alle angemeldeten Benutzer)

Kopf mit Typ, Gerät, Status, Meldung und letzter Abfrage; Zeitraum-Schalter **1 Stunde / 24 Stunden / 7 Tage / 30 Tage** (`?range=1h|24h|7d|30d`,
ungültige Werte ergeben 24 Stunden); Graph; Tabelle mit dem letzten Wert je Kanal; Name (Umbenennen, nur Administratoren); Schwellwerte; die letzten 20 Statuswechsel dieses Sensors.
Die Sensorliste eines Geräts (`/devices/<id>/sensors`) verlinkt jeden Sensor dorthin.

## Graph (uPlot)

- **uPlot 1.6.32** (MIT) liegt unverändert unter `static/vendor/uplot/` (mit Lizenz, Quelle und Prüfsumme in der dortigen `README.md`) und wird mit dem Binary
  ausgeliefert – kein CDN, kein npm. Eigener Code: `static/chart.js` (kein Inline-Skript, die CSP bleibt `default-src 'self'`).
- Daten kommen als JSON von `/sensors/<id>/data?range=…`: `{"range","step_secs","interval_secs","ts":[Sekunden…],"series":[{"key","label","unit","values":[…|null]}]}`.
  Alle Kanäle teilen die Zeitachse; fehlt ein Wert, ist er `null` (Lücke im Graph, keine Linie).
- Je Einheit ein Diagramm (Eingehend und Ausgehend teilen sich eines). Anzeige in der **Zeitzone des Browsers** (`static/chart.js`), 24-Stunden-Uhr, deutsche Zahlen und Einheiten
  (bit/s, kbit/s, Mbit/s, Gbit/s). Der Graph lädt alle 30 s nach.
- Herkunft der Punkte: 1 h und 24 h Rohwerte, 7 d Mittelwerte in 10-Minuten-Blöcken, 30 d Stundenwerte aus `samples_1h` (solange noch keine Rollups
  existieren, Stundenmittel direkt aus den Rohdaten). Höchstens 20 000 Punkte je Kanal.
- Statische Dateien haben `Cache-Control: no-cache` und ein ETag (Inhalts-Hash): Der Browser fragt bei jedem Laden kurz nach und bekommt `304 Nicht geändert`,
  solange sich nichts geändert hat. Nach einem Update gelten neue Dateien daher sofort. (Das `?v=2` an den Adressen umgeht einmalig die alten Ein-Stunden-Caches.)

## Tabelle „Messwerte“

Unter dem Graphen steht – wie die „Livedaten“-Tabelle von PRTG – eine Zeile je Messwert, **neueste oben**, für den gewählten Zeitraum
(1 Std. und 24 Std.: jede Abfrage; 7 Tage: 10-Minuten-Mittel, Spalte „Zeitraum ab“; 30 Tage: Stundenmittel).

- Traffic-Kanäle haben je Richtung zwei Spalten: **Volumen** (Datenmenge im Abschnitt, dezimal: 1 MB = 1 000 000 Byte) und **Geschwindigkeit**.
  Das Volumen ist Rate × Dauer; bei Rohwerten ist die Dauer der Abstand zur vorigen Abfrage, höchstens ein Abfrageintervall
  (nach einem Neustart oder einer verlorenen Abfrage würde ein längerer Abstand das Volumen sonst aufblähen), bei Mitteln die Blocklänge. Es ist damit eine Näherung;
  PRTG rechnet mit den Zählerständen selbst.
- Darüber stehen **Summe** (nur Volumina), **Mittelwert**, **Minimum** und **Maximum**, jeweils „aus N Werten“. Statuskanäle zeigen den Namen (`up`, `down` …).
- Angezeigt werden die neuesten **50** Zeilen; über „Zeilen“ lassen sich **100** oder **500** wählen (der Browser merkt sich die Wahl).
  Summe, Mittelwert, Minimum und Maximum gelten immer für den ganzen Zeitraum. Die Tabelle aktualisiert sich mit dem Graphen alle 30 s (`static/samples.js`, gemeinsame Formatierung in `static/format.js`).
- Noch nicht wie bei PRTG: Spalten für Ausfallzeit und Abdeckung (fehlgeschlagene Abfragen werden nicht als Zeilen gespeichert; Lücken sieht man im Graphen).

## Zeitangaben in Tabellen

Der Server schreibt jeden Zeitpunkt als `<time class="ts" datetime="2026-10-15T19:32:10Z">2026-10-15 19:32:10 UTC</time>` (`web::time_html`). `static/time.js` (auf jeder Seite geladen)
ersetzt den Text im Browser durch die lokale Zeit im Format `15.10.2026 21:32:10` und legt die UTC-Angabe als Tooltip ab. Ohne JavaScript bleibt die UTC-Angabe stehen.
Betroffen: letzte Abfrage und Meldungen der Sensoren, Statuswechsel und Sendeversuche. **E-Mails** nennen die Zeit weiterhin ausdrücklich in UTC.

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
