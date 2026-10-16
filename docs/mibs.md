# MIB-Dateien

netmon versteht MIB-Dateien. Damit lassen sich eigene OID-Sensoren **per Name** anlegen (`tpSysMonitorCpu5Seconds.1` statt
`1.3.6.1.4.1.11863.6.4.1.1.1.1.2.1`), und Auswertung, Einheit und Klartext für Statuswerte kommen aus der MIB.

## Mitgelieferte Standard-MIBs

Im Binary stecken die gängigen Standard-MIBs (Quellen und Lizenz in `mibs/README.md`): SNMPv2-SMI/-TC/-CONF/-MIB, RFC1155-SMI, RFC-1215,
RFC1213-MIB (MIB-II), IF-MIB, IANAifType-MIB, SNMP-FRAMEWORK-MIB, HOST-RESOURCES-MIB (CPU, Speicher, Datenträger vieler Geräte),
INET-ADDRESS-MIB, ENTITY-MIB und ENTITY-SENSOR-MIB (Temperaturen u. Ä.). Sie sind immer geladen und können nicht durch Uploads ersetzt werden.

## Hersteller-MIBs hochladen (`/mibs`, nur Administratoren)

- Unter **MIB-Dateien** eine oder viele Dateien auf einmal auswählen (z. B. den ganzen entpackten Ordner des Herstellers) und hochladen.
  Für die TP-Link-Switches ist das das Paket „TP-Link_L2 Switch_MIB“ (`L2-tplinkMibs.zip`, 104 Dateien) von der Omada-Supportseite des Modells.
- Jede Datei wird geprüft: Text mit mindestens einem Modul („… DEFINITIONS ::= BEGIN“), höchstens 2 MB je Datei, 300 Dateien und 20 MB je Upload.
  Dateien, die nur ein mitgeliefertes Modul enthalten (z. B. TP-Links `RFC1213-MIB.mib`), werden übersprungen.
- Enthält eine neue Datei ein Modul, das schon geladen ist, **ersetzt** sie die ältere Fassung.
- Die Liste zeigt je Datei die Module mit Objektanzahl und Problemen: **fehlt: X-MIB** (eine importierte Datei fehlt noch – nachladen),
  **n Definitionen nicht auflösbar** (z. B. Verweis auf einen nirgends definierten Knoten – so bei 19 Produktkennungen in TP-Links `tplink-products.mib`,
  für das Monitoring ohne Bedeutung).
- Löschen entfernt die Datei; der Index wird sofort neu aufgebaut. Bestehende Sensoren bleiben unverändert (sie speichern die numerische OID),
  nur Namen und Klartext in der Anzeige fallen dann weg.
- **Suche** auf derselben Seite: nach Name oder Begriff in der Beschreibung; Treffer mit OID, Art (Einzelwert, Tabellenspalte, Tabelle, …),
  Typ, Einheit, Zugriff und Beschreibung. Abfragbare Werte stehen oben und zeigen den Eintrag für das OID-Feld.

## Eigene OID per Name

Im Formular „Sensor hinzufügen“ (Typ „Eigene OID“; beim Tippen ins OID-Feld wird der Typ automatisch gewählt):

| Eingabe | Bedeutung |
|---|---|
| `tpSysMonitorCpu5Seconds.1` | Name plus Index (Tabellenspalte; Index = Zeile, hier Unit 1) |
| `sysUpTime` | Einzelwert; `.0` wird automatisch angehängt |
| `IF-MIB::ifInErrors.3` | mit Modul, falls ein Name in mehreren MIBs mit verschiedener OID vorkommt |
| `1.3.6.1.2.1.1.3.0` | weiterhin numerisch; ist die OID bekannt, wird sie trotzdem benannt |

- Beim Tippen erscheinen Vorschläge; bei einem bekannten Namen zeigt die Zeile darunter Modul, Art, Typ, Einheit, OID und Beschreibung
  (bei Tabellenspalten auch, welcher Index anzuhängen ist).
- **Auswertung „Automatisch“**: Zähler (`Counter32/64`) werden als Rate pro Sekunde gemessen, alles andere als Wert. Wert/Zähler lässt sich weiterhin fest wählen.
- **Einheit**: leer = `UNITS` aus der MIB (falls vorhanden); sonst frei eintragen (z. B. `%`, höchstens 24 Zeichen). Sie steht hinter jedem Wert.
- **Name**: leer = Objektname mit Index (z. B. „tpSysMonitorCpu5Seconds.1“).
- Klare Fehlermeldungen für Tabellen/Zeilen/Knoten („kein abfragbarer Wert“), Spalten ohne Index, nicht lesbare Objekte und unbekannte Namen.

## Anzeige

- Sensorliste und Detailseite nennen OID-Sensoren mit Namen und Modul („tpSysMonitorCpu5Seconds.1 (TPLINK-SYSMONITOR-MIB) · Wert, Einheit %“).
- **Statuswerte im Klartext**: Hat das Objekt eine Aufzählung (z. B. `ifOperStatus`: 1 = up, 2 = down …), zeigen Kachel, Graph-Achse und Tabelle
  „down (2)“. Das gilt auch für den eingebauten Interface-Status-Sensor.
- Noch nicht: Ein eigener OID-Sensor bewertet Aufzählungswerte nicht selbst (ein `ifOperStatus` von „down (2)“ ist „In Ordnung“, solange kein
  Schwellwert greift). Abhilfe vorerst über Schwellwerte, z. B. „Down über 1“ für ifOperStatus; besser ist der Interface-Status-Sensor.

## Technik

- `src/mib/lexer.rs`, `src/mib/parser.rs`: toleranter Parser für SMIv1/SMIv2 (Importe, OID-Zuweisungen, OBJECT-TYPE mit SYNTAX/UNITS/ACCESS/
  DESCRIPTION/INDEX/AUGMENTS, Textual Conventions, Aufzählungen; Makrodefinitionen, Compliance-Abschnitte u. Ä. werden übersprungen).
  Hersteller-Eigenheiten wie `n/a(0)` oder `power_user(1)` in Aufzählungen und Windows-Zeilenenden werden verkraftet.
- `src/mib/mod.rs`: Auflösung über Modulgrenzen (Importe, notfalls eindeutiger Name in einem anderen Modul), Basistypen über Textual Conventions,
  Tabellen/Zeilen/Spalten inkl. `AUGMENTS`, Suche, Name ↔ OID. Der Index liegt im Speicher und wird beim Start sowie nach jedem Upload/Löschen neu
  gebaut (Datenbank: nur die Dateitexte, Tabelle `mib_files`, Migration `0009`).
- Gegenprobe mit net-snmp: `dev/mib-oracle.sh dev/data/mibs/tplink/*.mib` vergleicht alle aufgelösten OIDs mit `snmptranslate`.
  Ergebnis mit den Standard- und TP-Link-MIBs: 2963 gemeinsame Namen, **0 Abweichungen**; netmon kennt 230 Objekte mehr, weil net-snmp zwei
  fehlerhafte TP-Link-Dateien abweist.
- Tests: Lexer/Parser (auch abgeschnittene und kaputte Eingaben), alle mitgelieferten MIBs fehlerfrei, bekannte OIDs/Typen/Aufzählungen, Namensauflösung,
  Upload als Multipart, Ersetzen, Löschen, Rechte, Sensoren per Name; die TP-Link-Dateien werden mitgetestet, wenn sie lokal unter `dev/data/mibs/tplink` liegen.
