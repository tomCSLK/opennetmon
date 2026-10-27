# Plan: opennetmon – SNMP-Monitor für Linux

> Dieser Plan ergänzt `CLAUDE.md`. Bei Widersprüchen gilt `CLAUDE.md`.
> Der Name ist **opennetmon** (festgelegt am 2026-10-19 mit neuem Logo; vom 2026-10-16 bis dahin „netmon“). „PRTG" ist eine Marke von Paessler; opennetmon ist nur vom Bedienkonzept inspiriert.

## Stand (2026-10-26, Release v0.4.4)

Alle sechs MVP-Schritte sind umgesetzt, dazu einiges darüber hinaus. opennetmon läuft seit v0.4.4 als `.deb` auf dem Ubuntu-Server des Nutzers;
die Instanz auf dem Laptop dient nur noch zum Testen. Als Nächstes (Wunsch 2026-10-26, siehe „Nächste Vorhaben“): **mobile Ansicht überarbeiten**
und **Abhängigkeiten wie in PRTG** (automatisch pausieren, wenn ein anderer Sensor ausfällt). Dazu Symbole statt Text bei den Zeilenaktionen
der Listen, wie im Gerätebaum. Weiterhin offen: Test mit dem **echten Mailserver** des Nutzers, ein Ende-zu-Ende-Integrationstest, Feinschliff
und eine **Update-Möglichkeit** (Zeitpunkt offen).

Umgesetzt (Details in `docs/`):

- **Geräte und Gruppen:** CRUD, hierarchische Gruppen, Verbindungstest, Geräte ohne SNMP (nur Ping; der ER605 läuft so), Gerätetypen fürs Symbol
  in der Übersicht, Reihenfolge automatisch natürlich sortiert oder von Hand (Ziehen, ↑/↓, Position) (`docs/devices-and-groups.md`)
- **SNMP-Profile:** v2c und v3 authNoPriv/authPriv, Secrets verschlüsselt (`docs/snmp-profiles.md`)
- **Sensoren:** Ping, Uptime, Interface-Traffic (64 Bit mit Fallback auf 32 Bit), Interface-Status, OID (Gauge/Counter); Interface-Discovery per Walk,
  Umbenennen, Mini-Graphen der letzten Stunde (`docs/sensors.md`)
- **Dieser Server:** CPU-Last, Load, Arbeitsspeicher und Swap, Laufzeit, Dateisysteme, Temperaturen (hwmon) und Netzwerk-Traffic des Rechners, auf
  dem opennetmon läuft, direkt aus `/proc`, `/sys` und `statvfs` (kein snmpd, keine Zugangsdaten); Auswahl per Häkchen, Grenzen für Speicher,
  Dateisysteme und Temperaturen beim Anlegen (`docs/sensors.md`, „Dieser Server“)
- **MIBs:** Import mit Parser und Standard-MIBs, OID per Name; „Werte suchen“ (PoE je Port, CPU, Speicher, Temperaturen, Lüfter; neue Sensoren pausiert)
  (`docs/mibs.md`, getestet am SG2008P)
- **Gerätevorlagen:** aus einem eingerichteten Gerät speichern, beim Anlegen automatisch per `sysObjectID` oder später per Knopf anwenden, nur Fehlendes
  anlegen (`docs/device-templates.md`, getestet SG2008 → SG2008)
- **Poller und Status:** Scheduler mit Jitter, Semaphore, Timeouts/Retries, Fail-fast bei geräteweitem SNMP-Fehler; Statuslogik, Ereignislog (`docs/poller.md`)
- **Schwellwerte und Graphen:** je Kanal warn/down über/unter, Bewertung von Aufzählungswerten (In Ordnung/Warnung/Ausfall je Wert, Vorschlag aus den
  Namen), Detailseite mit uPlot-Graph 1h/24h/7d/30d (bis 120 gemittelte Punkte), Messwerte-Tabelle wie die PRTG-Livedaten, Rollups und Retention
  (`docs/charts-and-thresholds.md`)
- **Übersicht:** Gerätebaum wie in PRTG (Linien, ⊟/⊞, Sensorplaketten je Zustand, Summen je Teilbaum), Kacheln öffnen sortierbare Statuslisten,
  „Aktualisieren“ mit automatischem Neuladen alle 60 s; **Gruppenseite** je Gruppe (Klick auf den Namen; Teilbaum, eigene Kacheln mit auf die Gruppe
  gefilterten Statuslisten, Aktionen, Rücksprung der Formulare dorthin; 0.4.2)
- **Pausieren wie in PRTG:** Gruppen, Geräte und Sensoren bis zum Fortsetzen oder 15 min bis 1 Woche, mit Grund; Vererbung nach unten, Ablauf durch den
  Poller, Neustart ohne alten Zustand (keine nachträgliche Erholungsmail) (`docs/devices-and-groups.md`, „Pausieren“)
- **Protokoll** (`/log`): Warnungen, Ausfälle und ihr Ende, gemischt mit Pausen (je die neuesten 500)
- **Benachrichtigung:** E-Mail bei Statuswechsel, Einstellungen in der Oberfläche, Testmail, Sendelog (`docs/notifications.md`; getestet nur gegen
  Test-SMTP-Server)
- **Benutzer:** Login, Rollen Admin/Leser, Benutzerverwaltung in der Oberfläche, „Mein Konto“, Begrenzung der Anmeldeversuche (`docs/users-and-login.md`)
- **Mehrsprachigkeit:** Oberfläche Deutsch/Englisch je Benutzer, Mails in einer einstellbaren Sprache, README auf Englisch (`docs/translations.md`)
- **Oberfläche:** Seitenleiste, Kacheln, hell/dunkel, alle Zeiten in der Browser-Zeitzone; rein serverseitig gerendert, wenige eigene Skripte in `static/`
- **Betrieb:** statisches musl-Binary, systemd-Unit mit Hardening, Installer, `.deb`-Paket, eingebautes HTTPS, Backup mit Timer, Proxy-Beispiele
  (`docs/operations.md`)

Releases (x86_64; bis v0.4.3 Vorabversionen, ab v0.4.4 reguläre Releases): **v0.1.0** (2026-10-19), **v0.2.0** (2026-10-20, Mehrsprachigkeit, .deb), **v0.3.0** (2026-10-20, eingebautes HTTPS),
**v0.3.1** (2026-10-20, Statuslisten hinter den Kacheln), **v0.4.0** (2026-10-21, Pausieren, Protokoll, Gerätebaum mit Gerätetypen),
**v0.4.1** (2026-10-21, Sensoren „Dieser Server“), **v0.4.2** (2026-10-22, Gruppenseiten, Dokumentation auf Englisch),
**v0.4.3** (2026-10-23, Intervall je Sensor, Messwerttabelle mit Rohwerten), **v0.4.4** (2026-10-24, Abmelden ohne Aktivität, MIB-Dateien nach Hersteller, Suche und Sortierung im Protokoll).

## Kontext

PRTG gibt es nur für Windows. Ziel ist eine Linux-native Anwendung mit ähnlichem Bedienkonzept, die Netzwerkgeräte
agentlos per SNMP überwacht. Zielumgebung: privates Netz mit TP-Link Omada: Gateway ER605, Switches 2× SG2008 und 1× SG2008P,
Access Points 1× EAP225 und 2× EAP653 (7 Geräte). SNMP ist im Controller site-weit nur als SNMPv3 authNoPriv/MD5 aktiviert (v1/v2c aus).
Die Omada-Controller-API ist nicht Teil des MVP.
Entwickelt wird auf Zorin OS, betrieben auf einem headless Ubuntu Server; Bedienung nur per Browser.
Der Server hat Internetzugang (geklärt 2026-10-21) und ist nicht allein für opennetmon da: dort laufen später weitere Programme, die das Internet
brauchen. opennetmon muss sich deshalb Ports und Ressourcen teilen (Standardport 8443) und darf nichts am System verändern, was andere Dienste betrifft.

## Leitentscheidungen (aus CLAUDE.md)

| Thema | Entscheidung |
|---|---|
| Sprache | **Rust** (Edition 2024, stable), entschieden am 2026-10-15 nach dem Spike (`docs/decisions/0001-language.md`) |
| Web | `axum`, `tower-http`, `askama` (serverseitiges HTML), uPlot für Graphen (lokal vendort, per `rust-embed` ins Binary, kein CDN, kein npm); kein htmx (2026-10-21 gestrichen: die Oberfläche kommt ohne aus, das Neuladen erledigt `static/refresh.js`) |
| SNMP | `snmp2` (Features `tokio`, `crypto-rust`, `heap_buffers`); v2c und v3 (authNoPriv/authPriv); Omada-Geräte nutzen v3 authNoPriv/MD5; ausschließlich lesend |
| Ping | `surge-ping`, bevorzugt unprivilegierte ICMP-Sockets (`ping_group_range`) |
| DB | SQLite über `sqlx`, WAL, Migrations in `migrations/` |
| Auth | Lokale Benutzer, Rollen Admin/Leser, argon2, serverseitige Sitzungen, CSRF |
| Secrets | SNMP- und SMTP-Passwörter mit XChaCha20-Poly1305 verschlüsselt, Schlüsseldatei 0600 (`opennetmon key generate`) |
| Übersetzungen | Fluent (`fluent-bundle`), Kataloge `locales/<code>/*.ftl`, Deutsch Ausgangssprache, Englisch mitgepflegt |
| Lizenz | **AGPL-3.0-or-later**, Copyright Thomas Cieslik; Quellcode-Link in der Oberfläche (Abschnitt 13), Fremdlizenzen in `THIRD-PARTY-LICENSES.txt` (`dev/third-party-licenses.py`) |
| Betrieb | Statisches musl-Binary, systemd-Unit, .deb-Paket; eingebautes HTTPS auf `0.0.0.0:8443` (ausgelieferte Config, seit 0.3.0), alternativ Reverse Proxy (Caddy/nginx) vor `127.0.0.1:8080` |

## PRTG-Funktionsprinzip, das nachgebaut wird

- Objektbaum: **Gruppe → Gerät → Sensor → Kanal**. Ein Sensor ist eine Messung (z. B. Traffic Port 1:24),
  ein Kanal ein einzelner Messwert (z. B. In-/Out-Bit/s).
- Jeder Sensor wird zyklisch abgefragt (Gerät hat Polling-Intervall, Standard 60 s).
- Messwerte werden als Zeitreihe gespeichert; Graphen für 1h/24h/7d/30d.
- Status: `Up`, `Warning`, `Down`, `Unknown`, `Paused`. Sensorstatus aus Kanälen (Schwellwerte),
  Gerätestatus aus Sensoren (schlechtester Status), Gruppenstatus analog.
- Timeout oder SNMP-Fehler ergibt beim ersten Mal `Warning`, ab dem zweiten Mal in Folge `Down`.
- Pausieren vererbt sich: eine pausierte Gruppe oder ein pausiertes Gerät pausiert alles darunter.
- Beim Anlegen eines Geräts: Interface-Discovery per Walk, Nutzer wählt die Sensoren aus (kein Subnetz-Scan).

## Ergebnisse Phase 0 (Spike)

- Alle drei Switches und alle drei Access Points antworten auf SNMPv3 **authNoPriv/MD5**; authPriv wird abgelehnt.
  Der ER605 antwortet nicht auf SNMP (nur Ping-Überwachung möglich).
- Switches: `ifXTable` mit 64-Bit-Countern (9 Interfaces, nicht fortlaufende Indizes wie 49153).
  Access Points: nur `ifTable` mit 32-Bit-Countern, Wrap bei 1 Gbit/s nach rund 34 Sekunden.
- `snmp2` hat keinen eingebauten Timeout, jede Anfrage braucht einen eigenen `tokio::time::timeout`.
- Lokaler `snmpd` als Testagent: v2c (nur Systemgruppe sichtbar), v3 SHA-256/AES-128 (Anleitung: `docs/dev-testagent.md`).

## Architektur

Ein Binary, ein Prozess. Module und ihre Regeln stehen in `CLAUDE.md` (Abschnitt „Architektur“).

### Datenmodell

Migrations `0001`–`0018` (dazu einmalige Neuaufbauten von `devices` und `sensors` in `storage::migrate`): `meta`, `users`, `sessions`, `credential_profiles` (Secrets verschlüsselt), `groups` (hierarchisch, höchstens 8 Ebenen),
`devices` (Profil optional für Geräte ohne SNMP, Gerätetyp `kind`), `sensors`, `channels` (Einheit, Schwellwerte), `value_states` (Bewertung von
Aufzählungswerten), `samples`, `samples_1h`, `samples_1d`, `sensor_status`, `events`, `mail_settings`, `notifications`, `mib_files`,
`device_templates`, `pause_log`; `sensors.target` für die Sensoren „Dieser Server“; Pausen als Spalten an `groups`/`devices` (`paused`, `pause_until_ms`, `pause_reason`), Spalte `position` für die Handordnung, `sensors.interval_secs` (Intervall je Sensor), `sessions.last_seen_at` und `users.stay_signed_in` (Abmelden ohne Aktivität).

Retention (konfigurierbar): Rohdaten 7 Tage, Stundenwerte 1 Jahr, Tageswerte unbegrenzt.
Counter → Rate per Differenz/Zeitdelta; bei Wrap oder Reset (z. B. `sysUpTime` kleiner) wird **kein** Wert geschrieben.
Zeitstempel intern UTC, Anzeige in der Zeitzone des Browsers.

## Projektstruktur (Stand)

```
opennetmon/
  Cargo.toml, CLAUDE.md, PLAN.md, README.md, CONTRIBUTING.md, SECURITY.md, LICENSE, THIRD-PARTY-LICENSES.txt
  .github/              # CI (workflows/ci.yml), Issue- und Pull-Request-Vorlagen
  src/
    main.rs             # CLI: serve (Standard), user add|passwd|list, key generate, backup, tls generate
    lib.rs, config.rs, error.rs, i18n.rs, tls.rs
    auth/               # Rollen, Argon2, Tokens, Validierung, throttle (Anmeldeversuche)
    crypto.rs           # SecretBox (XChaCha20-Poly1305), Schlüsseldatei
    credentials.rs      # SNMP-Profile
    devices.rs          # Geräte, Gruppen, Gerätetypen
    snmp/               # SnmpClient, probe; testagent.rs (Fake-v2c-Agent, nur Tests)
    discovery.rs        # Interface-Discovery
    mib/                # Lexer, Parser, Index, catalog (bekannte Tabellen)
    value_discovery.rs  # „Werte suchen“
    templates.rs        # Gerätevorlagen
    ordering.rs         # Anzeigereihenfolge
    pause.rs            # Pausen und ihre Vererbung
    sensors/            # Trait Sensor, ping, snmp, local (Dieser Server), spec, counter, format
    status.rs           # Schwellwerte, Bewertungen, Zustandsautomat
    poller.rs           # Supervisor, eine Aufgabe je Gerät
    maintenance.rs      # Rollups, Retention
    alerting.rs, mail.rs, mail/   # Benachrichtigung, SMTP (testsmtp.rs nur Tests)
    storage/            # Repository je Thema (users, credentials, groups, devices, sensors, status, pauses, measurements, mail, mibs, templates)
    web/                # Router, Handler je Seite, assets, testutil.rs (nur Tests)
  templates/            # askama
  static/               # app.css, eigene Skripte (theme, app, chart, refresh, order, samples, time, …), vendor/uplot
  locales/              # de/, en/ (Fluent)
  mibs/                 # mitgelieferte Standard-MIBs
  migrations/           # 0001–0016
  examples/snmp_probe.rs
  tests/                # snmp_local.rs (lokaler snmpd), omada_manual.rs (echte Hardware, #[ignore])
  deploy/               # Unit, Installer, Backup, sysctl, Proxy-Beispiele, deb/ (Maintainer-Skripte)
  dev/                  # config.toml, sync.sh (Drive nur CLAUDE.md und PLAN.md), release.sh, deb.sh, smtp_sink.py, mib-oracle.sh, third-party-licenses.py
  docs/                 # Dokumentation (Englisch, seit 2026-10-22), decisions/
```

## Umsetzungsphasen

0. ✔ **Spike:** `examples/snmp_probe` gegen lokalen Agenten und Omada-Geräte; Entscheidung Rust.
1. ✔ **Gerüst:** Config (TOML + ENV), Error-Typ mit `IntoResponse`, Logging, `/healthz`, Migrations, SQLite/WAL, Graceful Shutdown.
2. ✔ **MVP 1:** Benutzer, Login, Rollen, CSRF, Sicherheits-Header; SNMP-Profile mit verschlüsselten Secrets; Geräte und Gruppen mit Verbindungstest.
3. ✔ **MVP 2 und Poller:** `Sensor`-Trait, Counter-/Ratenlogik inkl. Wrap/Reset (32 und 64 Bit), fünf Sensortypen; Poller mit Jitter, Semaphore und
   Fail-fast, Statuslogik und Ereignislog.
4. ✔ **MVP 3:** Interface-Discovery per Walk mit Auswahl durch den Nutzer (Namen über `ifName`/`ifDescr`).
5. ✔ **MVP 4:** Schwellwerte je Kanal, Statusübersicht, Detailseite mit uPlot-Graph, Rollups und Retention.
6. ✔ **MVP 5:** Benutzerverwaltung in der Oberfläche, „Mein Konto“, Begrenzung der Anmeldeversuche.
7. ✔ **MVP 6:** E-Mail-Benachrichtigung bei Statuswechsel.
8. ✔ **MIBs, „Werte suchen“, Gerätevorlagen, Bewertung von Aufzählungswerten, Reihenfolge, Geräte ohne SNMP.**
9. ✔ **Mehrsprachigkeit** (Fluent, Oberfläche und Mails; README Englisch, `docs/` Deutsch).
10. ✔ **Betrieb:** musl-Binary, systemd-Unit mit Hardening, Installer, Backup, Proxy-Beispiele, `.deb`-Paket (im Podman-Container getestet).
11. ✔ **Eingebautes HTTPS** (0.3.0): rustls, selbst signiertes Zertifikat bei der Installation, Klartext-HTTP nur Weiterleitung.
12. ✔ **Statuslisten, Gerätebaum, Gerätetypen, Pausieren, Protokoll** (0.3.1 und 0.4.0).
12a. ✔ **Sensoren „Dieser Server“** (0.4.1): eingebauter Sensortyp statt snmpd auf dem Server.
12b. ✔ **Gruppenseiten** (0.4.2): eigene Seite je Gruppe, Statuslisten mit Gruppenfilter; Dokumentation auf Englisch.
13. **Offen:**
    - Installation auf dem echten Ubuntu Server (Paket) ist erledigt (Betrieb in `Serveradmin`); dort noch die Omada-Geräte einrichten
    - Benachrichtigung mit dem echten Mailserver des Nutzers testen
    - Integrationstest Ende zu Ende gegen den lokalen `snmpd`: Gerät anlegen → Discovery → Messwerte nach 1–2 Intervallen → Agent stoppen → `Down`
      → Agent starten → `Up`
    - Feinschliff nach den ersten Tagen im Betrieb
    - Sensoren „Dieser Server“ erweitern, wenn gewünscht: ausstehende Updates, „Neustart nötig“, Zustand wichtiger systemd-Dienste (bräuchte
      D-Bus bzw. apt-Daten und lockerere Rechte der Unit; bewusst noch nicht umgesetzt)
    - **Update-Möglichkeit** (Wunsch 2026-10-21, Zeitpunkt offen): heute per neuem `.deb` mit `apt install ./…deb` (Daten und Config bleiben).
      Später z. B. ein apt-Repository (Updates mit `apt upgrade`, setzt ein öffentliches Repo oder eigenen Paket-Server voraus) und/oder ein Hinweis
      in der Oberfläche, wenn auf GitHub eine neue Version erschienen ist. Ein Selbst-Update aus der Oberfläche heraus passt nicht zum Hardening
      (`ProtectSystem=strict`, eigener Benutzer ohne Rechte auf `/usr`)
    - Später: `.deb` für arm64, ein apt-Repository erst nach dem Öffentlichmachen

Nach jedem Schritt ein lauffähiger Stand und ein kleiner, thematisch abgeschlossener Commit.
**Vor jedem Commit und Push werden `CLAUDE.md` und `PLAN.md` auf den aktuellen Stand gebracht.**

## Nächste Vorhaben (A–C eingeplant 2026-10-26 und in v0.5.0; D geplant 2026-10-27, noch nicht umgesetzt)

### A. Mobile Ansicht überarbeiten – umgesetzt 2026-10-26 (in v0.5.0)

**Umgesetzt** (geprüft bei 360, 390 und 430 px, hell und dunkel, Desktop bei 1400 px unverändert; Beschreibung in `docs/devices-and-groups.md`,
„Phones“): Kacheln dreispaltig und kompakt, Baum ohne Zeilenaktionen und Adresszeile, Plaketten nur mit Zahl (≤560 px); Menü „Aktionen“
(`details.more-actions`, auf breiten Bildschirmen eine Knopfreihe, bis 640 px ein Dropdown); Tabellen als Blöcke (`table.m-cards` mit Layoutklasse
und Zellklassen statt `data-label`), sortierbare Spaltenköpfe werden zu einer Leiste mit Sortier-Links; Sensordetail mit Zeitraumwahl über die volle
Breite, Legende in einer Zeile, Messwerttabelle ohne Volumen und Summe; feste Grid-Zeilen, damit kurze Seiten die Kopfleiste nicht strecken.
Bewusst nicht angepasst: die Tabellen der Einrichtungsseiten (Discovery, Werte suchen, „Dieser Server“, Vorlagenregeln) scrollen weiter in ihrem Rahmen.

Ursprünglicher Befund:

Wunsch des Nutzers: Die Ansicht auf dem Handy „schaut noch nicht so gut aus“. Befund vom 2026-10-26 im Browser mit einer Kopie der Testdaten, bei
500 px Breite (schmaler lässt sich das Chrome-Fenster nicht ziehen; echte Handys haben 360–430 px, dort ist es noch enger):

1. **Übersicht und Statuslisten, Kacheln:** Die fünf Kacheln stehen zweispaltig und hoch und füllen den ganzen ersten Bildschirm, der Gerätebaum
   beginnt erst darunter. Ziel: eine kompakte Statusleiste (Zahl und Wort klein, eine oder zwei Zeilen).
2. **Gerätebaum:** Die Sensorplaketten überdecken den ersten Aktionsknopf (Graph-Symbol), Gerätenamen werden abgeschnitten („Switch Wohnen …“),
   jede Zeile hat drei Aktionsknöpfe. Ziel: auf dem Handy nur Plakette (kürzer, z. B. nur Zahl mit Symbol) und Name, Namen dürfen umbrechen;
   die Aktionen auf den Gruppen- und Geräteseiten statt in jeder Zeile.
3. **Werkzeugleisten** (Übersicht, Geräteseite, Sensordetail): Die Knöpfe brechen in bis zu drei Zeilen um (Gerät bearbeiten, Pausieren, Protokoll,
   Verbindung testen, Werte suchen, Interfaces suchen, Automatisch, Aktualisieren). Ziel: die wichtigste Aktion sichtbar, der Rest in einem Menü
   „Weitere Aktionen“ (`<details>`/`<summary>`, ohne neues Skript).
4. **Tabellen** (Sensoren eines Geräts, Statuslisten, Protokoll, Messwerte): breiter als der Bildschirm, nur waagerecht scrollbar. Ausgerechnet
   „Aktuell“, der Mini-Graph, die Aktionen, Gerät und Meldung bzw. die Raten der Messwerte liegen außerhalb des sichtbaren Bereichs.
   Ziel: unterhalb von etwa 640 px jede Zeile als Block (oben Status und Name, rechts der aktuelle Wert, darunter Gerät bzw. Meldung), per CSS-Grid
   mit `data-label` an den Zellen; die Sortierung über die Spaltenköpfe braucht dann einen Ersatz (z. B. Auswahlfeld „Sortieren nach“).
5. **Sensordetail:** Die Legende des Graphen steht untereinander (drei Zeilen), der Graph ist niedrig; die Messwerttabelle zeigt nur das Volumen.
   Ziel: Legende in einer Zeile, Graph über die volle Breite, in der Tabelle Zeit und Rate (Volumen erst beim Wischen oder umschaltbar).
6. **Formulare** (Gerät bearbeiten): in Ordnung, nur mitprüfen.
7. **Allgemein:** Touch-Ziele mindestens 44 px, Abstände und Schriftgrößen, Handy-Menü der Seitenleiste, hell und dunkel; prüfen bei 360, 390
   und 430 px (dafür eine lokale Testseite mit `<iframe>` fester Breite oder den Gerätemodus der Entwicklerwerkzeuge).

Umsetzung nur mit CSS in `static/app.css` und kleinen Template-Änderungen (Klassen, `data-label`, `<details>`); keine Inline-Styles (CSP), keine
neue Abhängigkeit. Seite für Seite in kleinen Commits, jede im Browser geprüft (Memory „Check UI in browser“).

### B. Abhängigkeiten wie in PRTG (automatisch pausieren) – umgesetzt 2026-10-27 (in v0.5.0)

**Umgesetzt** wie im Entwurf unten, mit den drei Entscheidungen des Nutzers (Beschreibung in `docs/devices-and-groups.md`, „Dependencies“;
Architektur in CLAUDE.md). Abweichungen und Ergänzungen zum Entwurf:

- Der Poller verwirft einen Fehlschlag (statt ihn zu zählen), solange der Sensor, von dem abhängig ist, selbst Fehlschläge hat oder noch nie
  abgefragt wurde (`Pauses::dependency_unsettled`); damit braucht der `Notifier` kein eigenes Wissen über Abhängigkeiten außer beim Versand
  (`alerting::deliver` verwirft Wechsel von Sensoren, die beim Versand durch Abhängigkeit pausiert sind).
- Der Zustand „Pausiert“ wird von `storage::pauses::sync` geschrieben (nach jedem Zyklus mit Änderungen und alle 15 s), nur für Sensoren mit
  vorhandenem Zustand; dort entstehen auch die Protokolleinträge (`dependency_log`).
- Formular `/dependency/<kind>/<id>` mit Hinweis auf den übernommenen Wert; Sensorauswahl nach Gerät gruppiert in der Reihenfolge des Gerätebaums (Geräte mit Gruppenpfad, `in_tree_order`), ohne den Sensor selbst.
- Im Browser geprüft (Kopie der Testdaten, hell und dunkel). Offen: Auf der Geräteseite bricht die Knopfleiste unterhalb von etwa 1570 px um.

Ursprünglicher Plan:

Wunsch des Nutzers (2026-10-26, präzisiert): Objekte sollen sich **in Abhängigkeit von einem anderen Sensor** automatisch pausieren, nicht nach
ihrem eigenen Wert. Beispiel: Fällt der Ping-Sensor des Routers aus, werden alle Sensoren der Switches pausiert, statt dass jeder einzelne als
Ausfall gemeldet wird. So bekommt man eine Mail für die Ursache (Router) statt einer je Sensor dahinter. Vorbild ist die Einstellung
„Abhängigkeit“ in PRTG.

Entwurf:

- **Einstellung je Gruppe, Gerät und Sensor**, wie in PRTG:
  - *vom übergeordneten Objekt übernehmen* (Standard; Vererbung Gruppe → Untergruppe → Gerät → Sensor, wie beim Pausieren)
  - *von einem Sensor abhängig:* Auswahl eines beliebigen anderen Sensors (Gerät, dann Sensor), z. B. der Ping des Routers für die Gruppe „Switche“
  - *keine Abhängigkeit* (beendet die Vererbung)
  - zusätzlich am Sensor: *Master-Sensor für das Gerät* (typisch der Ping-Sensor): fällt er aus, sind die übrigen Sensoren seines Geräts pausiert;
    er selbst wird weiter abgefragt
- **Auslöser:** Der Sensor, von dem abhängig ist, steht auf „Ausfall“ (oder ist selbst pausiert, von Hand oder durch Abhängigkeit, damit Ketten
  wie Router → Switch → Access Point funktionieren). „Warnung“ allein löst nicht aus.
- **Wirkung:** Die abhängigen Objekte stehen auf „Pausiert“ mit dem Grund „durch Abhängigkeit von Ping (Router ER605)“, werden nicht abgefragt und
  senden keine Mails. Der Sensor, von dem abhängig ist, wird immer weiter abgefragt.
- **Rückkehr automatisch:** Steht der Sensor wieder auf „In Ordnung“, laufen die abhängigen Objekte von selbst weiter, wie nach einer Pause von Hand
  ohne alten Zustand (keine nachträgliche Erholungsmail). Danach eine einstellbare Verzögerung (PRTG: „Abhängigkeitsverzögerung“), damit z. B. ein
  Switch nach dem Router-Neustart erst hochfahren kann.
- **Wettlauf vermeiden:** Router und Switch werden von verschiedenen Geräteaufgaben abgefragt. Antwortet der Switch nicht, während der Router-Ping
  schon fehlschlägt (erst „Warnung“, ab der zweiten Abfrage „Ausfall“), darf der Switch keinen Ausfall melden: Fehlschläge abhängiger Sensoren
  zählen nicht, solange der Sensor, von dem sie abhängen, nicht „In Ordnung“ ist; der `Notifier` verwirft gesammelte Wechsel von Sensoren, die
  innerhalb seines 10-s-Fensters durch Abhängigkeit pausiert wurden.
- **Umsetzung:** Die Abhängigkeit wird wie die Pausen-Vererbung berechnet, nicht als Pause gespeichert: `pause::Pauses` bekommt eine Quelle
  `Source::Dependency` (Sensor-ID und Name), `storage::pauses::load` liest dazu den Zustand der Sensoren, von denen abhängig ist, aus
  `sensor_status`. Migration `0019`: Spalten `dependency` (`parent`/`sensor`/`none`) und `dependency_sensor_id` (`ON DELETE SET NULL` →
  wieder „übernehmen“) an `groups`, `devices` und `sensors`, dazu `sensors.dependency_master`; Zyklen (A hängt von B ab, B von A, auch über
  Vererbung) beim Speichern ablehnen.
- **Anzeige:** Status „Pausiert“ mit Grund überall, wo heute Pausen erscheinen (Baum, Statuslisten, Geräte- und Sensorseite); im Protokoll ein
  Eintrag je Auslösung und Rückkehr („Abhängigkeit aktiv: 14 Sensoren pausiert“) statt einer Zeile je Sensor. Gerätevorlagen übernehmen die
  Master-Markierung.
- **Tests:** reine Logik (Vererbung, Auslösung, Ketten, Zyklen) als Unit-Tests, Poller mit dem Fake-Agenten (Router-Ping fällt aus → Switch
  pausiert, keine Mail für den Switch → Router zurück → Switch läuft weiter), Handler, Übersetzungen DE/EN; Doku in `docs/devices-and-groups.md`.

Entschieden vom Nutzer (2026-10-26), alle drei mit „ja“:

1. **Mail:** Die Ausfallmail des Sensors, von dem abhängig ist, nennt in einer Zeile, was dadurch pausiert wurde (z. B. „14 Sensoren an 3 Geräten
   pausiert“).
2. **Verzögerung nach der Rückkehr** wird gleich mit eingebaut: je Gruppe, Gerät und Sensor in Sekunden (Standard 0, vererbt wie die
   Abhängigkeit); Spalte `dependency_delay_secs` in Migration `0019`.
3. **Pause von Hand:** Ist der Sensor, von dem abhängig ist, von Hand pausiert, sind die abhängigen Objekte ebenfalls pausiert.

### C. Symbole statt Text bei den Zeilenaktionen – umgesetzt 2026-10-26 (in v0.5.0)

**Umgesetzt:** gemeinsame Makros in `templates/icons.html` (Stift, Papierkorb, Pause/Fortsetzen, Reihenfolge, Menüknopf), genutzt vom Baum und den
Listen der Tabelle unten. Der Papierkorb der Sensorliste führt auf den Abschnitt „Sensor löschen“ am Ende der Detailseite (`#delete`, Häkchen
`confirm=yes`, sonst 422); MIB-Dateien werden weiter sofort gelöscht, weil sie sich erneut hochladen lassen.

Ursprünglicher Plan:

Wunsch des Nutzers (2026-10-26): Die Textknöpfe rechts neben den Mini-Graphen der Sensorliste („Pausieren“, „Löschen“) und ähnliche Knöpfe
in anderen Listen durch Symbole ersetzen, wie im Gerätebaum der Übersicht (`templates/tree.html`: `ghost small icon-only`, Inline-SVG, Name als
`title` und `aria-label`). Gehört zur mobilen Ansicht (A), weil die Textknöpfe die Tabellen verbreitern.

Bestand (Textknöpfe in Tabellenzeilen bzw. Kartenköpfen):

| Seite | Knopf | Symbol |
|---|---|---|
| Sensoren eines Geräts (`sensors.html`), je Zeile | Pausieren, Löschen | ⏸ wie im Baum (bei pausierten ▶ Fortsetzen), Papierkorb |
| Sensoren eines Geräts, Kopf der Karte | Reihenfolge ändern | ↕ wie im Baum |
| SNMP-Profile (`credentials_list.html`), Benutzer (`users.html`) | Bearbeiten | Stift wie im Baum |
| MIB-Dateien (`mibs.html`) | Löschen | Papierkorb |

- Die SVG-Symbole und `pause_action` kommen aus gemeinsamen Makros (heute nur in `tree.html`), damit Baum und Listen dieselben Symbole nutzen;
  `aria-label` mit dem Namen des Objekts (wie `edit-named`), neue Texte dafür in beiden Katalogen.
- Auf dem Handy mindestens 44 px Tippfläche (siehe A, Punkt 7).
- **Löschen absichern:** Ein Sensor wird heute mit einem Klick auf „Löschen“ samt Messwerten sofort gelöscht, ohne Rückfrage. Mit einem kleinen
  Symbol, gerade auf dem Handy, wäre das zu leicht versehentlich ausgelöst. Deshalb führt der Papierkorb auf eine Bestätigung, wie beim Löschen von
  Geräten und Gruppen (Häkchen „wirklich löschen“). Bei den MIB-Dateien ebenso prüfen.
- Die großen Knöpfe in den Werkzeugleisten oben auf den Seiten (Gerät bearbeiten, Verbindung testen, Werte suchen …) behalten ihren Text; für
  das Handy kommen sie in das Menü „Weitere Aktionen“ (A, Punkt 3).

### D. Knopfleisten der Seitenköpfe: Alltag sichtbar, Einrichtung im Menü – geplant 2026-10-27 (noch nicht umgesetzt)

Befund: Auf der Geräteseite stehen für Admins bis zu neun Knöpfe plus „Automatisch“/„Aktualisieren“. Zwischen 641 und etwa 1570 px (die meisten
Laptops) bricht die Reihe in zwei Zeilen um; seit dem Knopf „Abhängigkeit“ (v0.5.0) schon unterhalb von etwa 1570 px. Bis 640 px stecken die Knöpfe
heute schon im Menü „⋯ Aktionen“ (`details.more-actions`).

Entschieden vom Nutzer (2026-10-27): **Variante 1**, also aufteilen. Was man im Alltag braucht, bleibt sichtbar. Die Einrichtung (ist eher eine
einmalige Sache) kommt auf jeder Breite in das aufklappbare Menü „⋯ Aktionen“. Gilt einheitlich für Geräte-, Gruppen- und Sensorseite. Verworfen
wurden ein breiterer Umschaltpunkt nur für die Geräteseite (dann wären auf dem Laptop auch Pausieren und Protokoll versteckt) und ein Skript, das
überzählige Knöpfe automatisch ins Menü schiebt (mehr Code, die Leiste springt).

| Seite | Sichtbar | Im Menü „⋯ Aktionen“ |
|---|---|---|
| Gerät (`sensors.html`) | Pausieren/Fortsetzen, Protokoll | Gerät bearbeiten, Abhängigkeit, Werte dieses Servers, Verbindung testen, Werte suchen, Interfaces suchen |
| Gruppe (`group.html`) | Pausieren/Fortsetzen (ein Protokoll je Gruppe gibt es bewusst nicht) | Gruppe bearbeiten, Abhängigkeit, Reihenfolge ändern, Neue Untergruppe, Neues Gerät |
| Sensor (`sensor_detail.html`) | Pausieren/Fortsetzen, Protokoll | Abhängigkeit, Sensor löschen |

- Rechts daneben bleiben wie bisher „Automatisch“/„Aktualisieren“ (Gruppe, Gerät, Sensor), auf der Sensorseite die Zeitraumwahl an ihrem Platz.
- Leser (keine Admins) sehen weiter nur Protokoll und Aktualisieren, ohne Menü.
- Auf dem Handy (bis 640 px) alles in einem Menü wie heute, auch die sichtbaren Knöpfe.
- Die Übersicht (`home.html`, drei Knöpfe) bricht nicht um und bleibt wie sie ist.
- Umsetzung: Alltagsknöpfe vor dem Menü, übrige in `details.more-actions` mit neuer Klasse (z. B. `.always`), die auf jeder Breite als Dropdown
  erscheint. CSS: die Dropdown-Regeln, die heute nur bis 640 px gelten, auch für `.always`. `static/app.js`: `.always` startet auf jeder Breite
  geschlossen und schließt bei Klick daneben bzw. Esc. Beschriftung „⋯ Aktionen“ gibt es schon (`actions`), keine neuen Katalogtexte nötig;
  keine Inline-Styles (CSP).
- Prüfung: Handler-Tests (welcher Knopf wo, Leser ohne Menü), Browser bei 1400, 1024, 800 und 390 px, hell und dunkel (Wegwerfkopie).
  Doku: `docs/devices-and-groups.md` („Phones“) und der Design-Absatz in CLAUDE.md.

## Bewusst nicht im MVP

SNMP-Traps, NetFlow/sFlow, verteilte Probes, Netzwerkkarten/Maps, SNMP SET, Mandantenfähigkeit, Omada-Controller-API,
Herstellermibs außer gezielt benötigten TP-Link-OIDs.
Zurückgestellt gegenüber früheren Ideen: SNMP v1, Subnetz-Scan, Docker-Betrieb, htmx.

Optionen, erst auf Nachfrage (z. B. ein GitHub-Issue):
- Knopf „Protokoll“ auch auf Gruppenseiten (`/log?group=<id>` mit allen Geräten der Gruppe und ihrer Untergruppen); am 2026-10-24 bewusst
  nicht umgesetzt, weil das Protokoll großer Gruppen wieder unübersichtlich würde. Geräte- und Sensorseiten haben den Knopf seit v0.4.4.

## Verifikation

- **Unit- und Handler-Tests** (487, `cargo test`): Counter-/Ratenberechnung (inkl. Wrap/Reset), Statuslogik, Pausen, Abhängigkeiten, Config, Auth, Krypto, Validierung,
  Storage, SNMP-Client gegen den Fake-Agenten, MIB-Parser, Übersetzungskataloge; Handler mit `tower::ServiceExt::oneshot` (Helfer in `web/testutil.rs`).
- **Integrationstests** gegen den lokalen `snmpd` (`tests/snmp_local.rs`, 5 Tests: Probe, falsche Community, v3 mit richtigem und falschem Passwort,
  64-Bit-Counter); der Ende-zu-Ende-Test steht noch aus (siehe „Offen“).
- **Gerätetests** gegen echte Omada-Hardware nur manuell (`tests/omada_manual.rs`, `#[ignore]`).
- **Oberfläche:** Änderungen im Browser mit einer Wegwerfkopie der Entwicklungsdaten prüfen, hell und dunkel.
- **Packaging:** `dev/release.sh` prüft Archiv, Installer-Probelauf und Inhalt des .deb; Paket im Podman-Container getestet (Installation, Update, remove, purge).
- **Abhängigkeiten:** `cargo audit` (RustSec; Ausnahmen mit Begründung in `.cargo/audit.toml`), dazu Dependabot auf GitHub.
- **Sicherheitstest 2026-10-27** (Code-Review und Angriffe auf eine Wegwerfkopie mit Admin- und Leserkonto): Rollen und CSRF auf allen Routen,
  Zugriff ohne Anmeldung, XSS über Namen auf allen Seiten und in den JSON-Endpunkten, Steuerzeichen, Pfade außerhalb von `/static`, Weiterleitungen,
  SQL, Sitzungen nach dem Abmelden, Wartezeit nach Fehlversuchen, TLS (nur 1.2/1.3) – in Ordnung. Behoben: MIB-Dateien mit langen Ketten von
  Definitionen (12 GB, danach Absturz bei jedem Start), gleichzeitige Anmeldeversuche (2,4 GB und 123 statt 10 Versuche), vier Crates mit
  RustSec-Meldungen, `Cache-Control: no-store` und strengere CSP. Danach erneut gemessen: höchstens ~200 MB, genau 10 Versuche je Adresse.
  Nicht geändert: Die Umleitung von HTTP auf HTTPS übernimmt einen gültigen Host-Header (trifft nur den Anfragenden selbst).
- **Vor jedem Abschluss:** `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.

## Sicherheit (Kurzfassung)

SNMP nur lesend; Secrets verschlüsselt in der DB (Schlüsseldatei 0600, Rechteprüfung beim Start), nie in Logs oder HTML; argon2;
`HttpOnly`/`SameSite=Strict`-Cookies, `Secure` per Config; CSRF-Schutz für alle POST-Formulare; serverseitige Validierung
von IP, OID, Intervallen; nur HTTPS (eingebaut, Klartext-HTTP nur Weiterleitung) oder Bind auf `127.0.0.1` hinter einem TLS-Proxy;
Begrenzung der Anmeldeversuche (`auth::throttle`).

## Offene Punkte (vom Nutzer zu klären)

- ER605: läuft als Gerät ohne SNMP (nur Ping); später zusätzlich über die Omada-API?
- Schlüsseldatei (`secret.key`): Sicherung getrennt von der Datenbank einplanen; Schlüsselrotation ist noch nicht umgesetzt
- Update-Möglichkeit: Weg und Zeitpunkt

Geklärt: Beiträge nach DCO (2026-10-21). Der Server hat Internetzugang und betreibt weitere Programme (2026-10-21).
