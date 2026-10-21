# Gerätevorlagen

Eine Gerätevorlage überträgt die Sensoren eines fertig eingerichteten Geräts auf weitere Geräte – etwa von einem Switch auf die anderen
Switches desselben Modells. Alle Vorlagenseiten sind nur für Administratoren.

## Vorlage erstellen: „Als Vorlage speichern“

Auf der Sensorseite eines Geräts im Bereich **Gerätevorlage** (`/devices/<id>/template/new`). opennetmon liest dabei vom Gerät die
Gerätekennung (`sysObjectID`) und die Interfaces.

| Teil | Möglichkeiten |
|---|---|
| Name | frei, eindeutig (Vorschlag: Name des Geräts) |
| Gerätekennung | vorbelegt mit der `sysObjectID` des Geräts, z. B. `1.3.6.1.4.1.11863.5.134` (SG2008) bzw. `…11863.5.154` (SG2008P); leer = nur von Hand wählbar |
| Traffic, Interface-Status | **alle aktiven Ports** des Zielgeräts, **alle Ports** (auch inaktive), **dieselben Ports** (nach Portname, mit den Sensornamen dieses Geräts) oder nicht übernehmen |
| Weitere Sensoren | Ping, Uptime und eigene Werte (z. B. PoE, CPU, Speicher) einzeln per Häkchen |

- Übernommen werden Name, Zustand (aktiv/pausiert), Faktor, Einheit, Schwellwerte und die Bewertung von Statuswerten. Bei „aktiven/allen Ports“ gelten die Schwellwerte nur,
  wenn alle Interface-Sensoren dieser Art am Quellgerät dieselben haben; aktiv werden die neuen Sensoren, wenn mindestens einer am Quellgerät aktiv ist.
- „Ports“ sind Interfaces der Typen Ethernet und WLAN – dieselbe Auswahl wie die Vorauswahl bei „Interfaces suchen“
  (bei TP-Link zählt dazu auch `Vlan-interface1`).
- „Dieselben Ports“ braucht die Interfaces des Quellgeräts; ist es nicht erreichbar, ist die Option gesperrt.
- Eine leere Vorlage wird abgelehnt.

## Vorlage anwenden

- **Beim Anlegen eines Geräts** (Formular „Neues Gerät“, Bereich „Gerätevorlage“, sobald es Vorlagen gibt):
  „Automatisch anhand der Gerätekennung erkennen“ (Standard), eine bestimmte Vorlage oder keine.
  Nach dem Speichern zeigt eine Ergebnisseite, was angelegt und was übersprungen wurde.
- **Später** mit „Vorlage anwenden“ auf der Sensorseite (`/devices/<id>/template/apply`); vorgewählt ist die zur Gerätekennung passende Vorlage,
  sonst die zuletzt verwendete. So lassen sich z. B. neu belegte Ports nachziehen.
- Angelegt wird nur, was fehlt: Ping/Uptime, wenn das Gerät noch keinen solchen Sensor hat; Interface-Sensoren, wenn es für das Interface
  noch keinen dieser Art gibt; eigene Werte, wenn die OID noch nicht überwacht wird. **Vorhandene Sensoren werden nie geändert oder gelöscht.**
- Eigene Werte werden vorher per GET geprüft; liefert das Gerät keine Zahl (z. B. kein PoE am SG2008), wird der Sensor **übersprungen** und
  auf der Ergebnisseite mit Grund genannt. Ebenso Ports von „dieselben Ports“, die es am Zielgerät nicht gibt.
- Belegte Namen werden durchnummeriert („Uptime (2)“).
- Ist das Gerät nicht erreichbar oder passt keine Vorlage zur Kennung, wird nichts angelegt; das Gerät selbst bleibt angelegt.
- Das Gerät merkt sich die zuletzt angewendete Vorlage („Eingerichtet mit …“ auf der Sensorseite).
- Zeitgrenzen wie bei der Interface-Suche: höchstens 10 s Timeout und 1 Wiederholung.

## Vorlagen verwalten (`/templates`, Menüpunkt „Gerätevorlagen“)

- Liste mit Inhalt, Gerätekennung und Zahl der damit eingerichteten Geräte.
- Detailseite: alle Regeln; Name und Gerätekennung änderbar; Löschen (Geräte behalten ihre Sensoren).
- Den Inhalt ändert man, indem man ein Gerät passend einrichtet und daraus eine neue Vorlage speichert.

## Gemessen an der echten Hardware

Vorlage aus Switch Büro (SG2008, Ping, Uptime, Traffic aktiver Ports) → neues Gerät für den Switch Keller mit „automatisch erkennen“:
Vorlage über `1.3.6.1.4.1.11863.5.134` gefunden, 7 Sensoren angelegt (Ping, Uptime, Traffic an 5 aktiven Interfaces), in etwa 2 s.

## Technik

- `src/templates.rs`: Regeln (`Body`, als JSON gespeichert), `build` (aus den Sensoren eines Geräts), `plan` (reine Logik: was fehlt, was
  übersprungen wird), `apply` (Interfaces lesen, OIDs in Blöcken zu 10 per GET prüfen, Sensoren mit Schwellwerten anlegen), `detect`.
- `src/storage/templates.rs`, Migration `0011`: Tabelle `device_templates` (Name, Gerätekennung, Quellgerät, Regeln) und `devices.template_id`
  (`ON DELETE SET NULL`). Neue Abhängigkeit `serde_json` (war über axum schon im Build).
- `src/web/templates.rs`, Templates `templates_list.html`, `template_detail.html`, `template_new.html`, `template_apply.html`, `template_result.html`.
- Tests: Regeln und Plan ohne Netz, Abfrage von Kennung und Werten gegen den Fake-Agenten, kompletter Ablauf über die Oberfläche
  (Vorlage speichern → Gerät mit Erkennung anlegen → erneut anwenden), Fehlerfälle und Rechte.
