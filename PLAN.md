# Plan: opennetmon – SNMP-Monitor für Linux

> Dieser Plan ergänzt `CLAUDE.md`. Bei Widersprüchen gilt `CLAUDE.md`.
> Der Name ist **opennetmon** (festgelegt am 2026-10-19 mit neuem Logo; vom 2026-10-16 bis dahin „netmon“). „PRTG" ist eine Marke von Paessler; opennetmon ist nur vom Bedienkonzept inspiriert.

## Stand (2026-10-27, Release v0.5.0)

Alle sechs MVP-Schritte sind umgesetzt, dazu einiges darüber hinaus. opennetmon läuft als `.deb` auf dem Ubuntu-Server des Nutzers (dort noch 0.4.4,
Update auf 0.5.0 offen); die Instanz auf dem Laptop dient nur noch zum Testen. In v0.5.0 kamen die mobile Ansicht, Symbole in den Listen,
**Abhängigkeiten wie in PRTG** und die Behebungen aus dem Sicherheitstest dazu (Vorhaben A–C); danach umgesetzt, noch nicht released: Knopfleisten
der Seitenköpfe (D) und aus den Vorhaben E–M (2026-10-27) Dienst-Sensoren (E), Verfügbarkeit und Berichte (G), `cargo audit` in der CI (H),
Update-Hinweis (I), Wartungsfenster (J) und das Änderungsprotokoll (M). **Später:** Push (F), 2FA (K), Schlüsselrotation (L). Weiterhin offen: Test mit dem
**echten Mailserver** des Nutzers und ein Ende-zu-Ende-Integrationstest.

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
    lib.rs, config.rs, error.rs, i18n.rs, tls.rs, http_client.rs, update_check.rs
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
    schedule.rs         # Wartungsfenster: Fenster, Zeitzone
    availability.rs     # Verfügbarkeit: Zählung, Rechnung, Geräte
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
  migrations/           # 0001–0021
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

## Nächste Vorhaben (A–C eingeplant 2026-10-26 und in v0.5.0; D umgesetzt 2026-10-27, noch nicht released)

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

### D. Knopfleisten der Seitenköpfe: Alltag sichtbar, Einrichtung im Menü – umgesetzt 2026-10-27 (noch nicht released)

**Umgesetzt** wie geplant, ohne doppelte Knöpfe im HTML: Das äußere `details.more-actions` bleibt das Handy-Menü, darin steckt
`details.more-actions.setup` mit der Einrichtung (breit ein Dropdown, rechtsbündig unter dem Knopf; auf dem Handy offen, ohne eigenen Knopf, unter
einer Linie). `static/app.js` behandelt je nach Breite das eine oder das andere als Menü; Klick daneben, Esc und Sprunglinks (`#delete`) schließen es.
Geprüft im Browser bei 1920 und 800 px sowie über eine lokale Testseite mit Iframes bei 360, 390 und 430 px, hell und dunkel; die Reihe passt bis
800 px in eine Zeile, bei 360 px rutscht „Automatisch“ wie bisher in eine zweite Zeile.

Plan:

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

## Weitere Vorhaben E–M (geplant 2026-10-27; E, G, H, I, J, M jetzt, F, K, L später)

Wunsch des Nutzers (2026-10-27): alle Ideen aus der Runde nach v0.5.0 planen. **Entschieden (2026-10-27): E, G, H, I, J und M werden jetzt
umgesetzt, F, K und L kommen in einer späteren Version.** Bei den Punkten **Zu entscheiden** am Ende jedes Vorhabens gilt jeweils die Empfehlung.
Reihenfolge der Umsetzung: H, E (bringt den HTTP-Client), I, J (bringt die Zeitzonen), G, M. Für alle gilt wie bisher: Texte in beiden Katalogen,
Doku in `docs/` (Englisch), README-Statustabelle, Handler- und Unit-Tests, UI im Browser prüfen (Wegwerfkopie, hell/dunkel, Handybreiten),
neue Abhängigkeiten nur mit Begründung und `THIRD-PARTY-LICENSES.txt`, Migrationen mit der nächsten freien Nummer. Releases bündeln mehrere Vorhaben.

**Was voneinander abhängt:**

- E (Dienst-Sensoren) bringt einen **HTTP(S)-Client** mit (`hyper-util` mit Feature `client-legacy`, `tokio-rustls`, `webpki-roots`, alle schon im
  Build, kein neues Crate). F (Push) und I (Update-Hinweis) brauchen ihn auch. Wer zuerst umgesetzt wird, baut ihn als eigenes Modul `http_client`.
- L (Schlüsselrotation) muss alle verschlüsselten Spalten kennen, auch die neuen aus F (Tokens) und K (TOTP-Secrets). Dafür bekommt L ein
  Verzeichnis der verschlüsselten Spalten mit einem Test gegen das Schema. Damit ist die Reihenfolge egal.
- G (Verfügbarkeit) zählt pausierte Zeit nicht mit. Damit zählen auch die Wartungsfenster aus J automatisch nicht mit.
- M (Änderungsprotokoll) protokolliert auch die Ereignisse aus K (2FA ein/aus, Zurücksetzen), wenn K schon da ist.

Aufwand grob: klein = bis ein halber Tag, mittel = ein bis zwei Tage.

| | Vorhaben | Aufwand | Nutzen im Heimnetz |
|---|---|---|---|
| E | Sensoren HTTP(S), TCP-Port, DNS, TLS-Zertifikat | mittel | hoch: Dienste statt nur Geräte |
| F | Push-Benachrichtigungen (ntfy, Gotify, Telegram, Webhook) | klein bis mittel | hoch |
| G | Verfügbarkeit in Prozent, Berichte | mittel | mittel |
| H | `cargo audit` in der CI | klein | Sicherheit |
| I | Hinweis auf neue Versionen | klein | mittel |
| J | Wartungsfenster nach Zeitplan | mittel | mittel |
| K | Zwei-Faktor-Anmeldung (TOTP) | mittel | Sicherheit (LAN, Port 8443) |
| L | Rotation von `secret.key` | klein bis mittel | Betrieb |
| M | Änderungsprotokoll (wer hat was geändert) | klein bis mittel | erst ab mehreren Admins |

### E. Sensoren für Dienste: HTTP(S), TCP-Port, DNS, TLS-Zertifikat – umgesetzt 2026-10-27

**Umgesetzt** wie geplant, alle vier Typen, Ablaufdatum über `x509-parser` (Beschreibung in `docs/sensors.md`, „Services“). Ergänzungen: Gerätevorlagen
übernehmen die Dienste, eine URL zum Quellgerät wird dabei zu `{host}`; die Texte für Geräte ohne SNMP heißen jetzt „Kein SNMP – Ping und Dienste“; ein
abgelaufenes Zertifikat meldet seine (negativen) Tage. Die Einstellungen ließen sich zunächst nach dem Anlegen nicht ändern; seit 2026-10-27 geht das über „Einstellungen“ (siehe unten, „Sensor bearbeiten“).
Geprüft: Neuaufbau der Tabelle auf einer Kopie der Testdaten (alle Spalten gleich, `integrity_check` ok), Formular im Browser hell/dunkel, echte Abfragen
einer frischen Instanz gegen ciesliks.de (HTTP 200, Zertifikat 61 Tage, Port 443), 1.1.1.1 (A, AAAA, falsche Adresse, unbekannter Name) und sich selbst.


Ziel: prüfen, ob ein Dienst antwortet, nicht nur ob das Gerät lebt. Beispiele: ciesliks.de, Home Assistant, beide AdGuard-Instanzen (DNS),
Omada-Controller, Traefik und das Let's-Encrypt-Wildcard-Zertifikat. Für Ziele ohne eigenes Gerät (z. B. ciesliks.de) legt man ein Gerät ohne SNMP an.

Neue Sensortypen, alle über den `Sensor`-Trait (keine Sonderfälle im Poller), Adresse vom Gerät (`ctx.host`):

| Typ | Einstellungen | Kanäle | Status |
|---|---|---|---|
| `http` | URL (Standard `https://<Gerät>/`), Methode GET/HEAD, erwartete Statuscodes (Standard 200–399), optional Schlüsselwort im Inhalt, Weiterleitungen folgen (aus/bis 5), Zertifikat prüfen (an; aus z. B. für den Omada-Controller mit eigenem Zertifikat) | Antwortzeit (ms), Statuscode | Ausfall bei Fehler, falschem Code oder fehlendem Schlüsselwort |
| `tcp` | Port | Verbindungszeit (ms) | Ausfall, wenn keine Verbindung |
| `dns` | Name, Typ A/AAAA, Server (Standard: das Gerät selbst, also z. B. die AdGuard-Instanz), optional erwartete Adresse | Antwortzeit (ms), Antwort passt (Statuswert) | Ausfall ohne Antwort oder bei falscher Adresse |
| `tls` | Port (Standard 443), Servername (SNI, Standard Gerät) | Tage bis Ablauf | Warnung unter 14 Tagen, Ausfall unter 3 Tagen (Standard, als Kanalgrenzen änderbar); Ausfall bei ungültiger Kette oder falschem Namen |

- **HTTP-Client** in einem eigenen Modul `http_client` (siehe oben): Timeout wie das Gerät, höchstens 1 MiB Inhalt lesen (nur für das Schlüsselwort),
  `User-Agent: opennetmon/<Version>`, keine Cookies, keine Zugangsdaten in Logs. Weil nur Admins Sensoren anlegen, ist eine beliebige URL in Ordnung.
- **DNS:** eigene kleine Abfrage über UDP (Anfrage bauen, Antwort für A/AAAA lesen, mit Tests gegen feste Pakete) statt eines Resolver-Crates. Wir
  brauchen nur „fragt diesen Server, kommt eine passende Antwort“. Das ist bewusst der Server des Geräts, nicht der des Systems, sonst merkt man den
  Ausfall einer AdGuard-Instanz nicht.
- **TLS:** Handshake mit `tokio-rustls`, Zertifikat aus der Verbindung. Für das Ablaufdatum braucht es einen X.509-Parser (Entscheidung 2).
- **Einheit** `Unit::Days` neu (Anzeige „Tage“).
- **Speicherung:** Die Einstellungen kommen als JSON in eine neue Spalte `sensors.params` (geprüft über `SensorSpec`), statt für jeden Typ eigene
  Spalten anzulegen. Weil die Spalte `kind` einen CHECK hat, baut `storage::migrate` die Tabelle `sensors` noch einmal neu auf (wie bei `local`),
  mit den neuen Typen und `params`.
- **Oberfläche:** „Sensor hinzufügen“ bekommt die vier Typen mit eigenem Formular. Die Detailseite zeigt beim TLS-Sensor zusätzlich Aussteller und
  Ablaufdatum. Gerätevorlagen übernehmen die neuen Typen.
- **Tests:** lokaler HTTP- und TLS-Testserver im Test (es gibt schon `rcgen` und `tokio-rustls`), DNS mit festen Paketen und einem UDP-Testserver.
  Doku in `docs/sensors.md`.

Zu entscheiden:

1. Alle vier Typen in einem Rutsch oder zuerst nur HTTP(S) und TLS? Empfehlung: alle vier, TCP und DNS sind klein, wenn der Rest steht.
2. Ablaufdatum lesen: neues Crate `x509-parser` (MIT/Apache-2.0, steht schon im Lockfile, wird aber nicht gebaut) oder selbst den kleinen Teil der
   DER-Struktur lesen. Empfehlung: `x509-parser`, weil eigenes ASN.1-Parsen fehleranfällig ist.

### F. Push-Benachrichtigungen zusätzlich zur Mail

Ziel: Ausfälle schneller aufs Handy bringen und nicht nur vom (noch nicht mit dem echten Server getesteten) Mailversand abhängen.

- **Kanäle**, beliebig viele, je an/aus: **ntfy** (Server-URL, Topic, optional Token; Priorität hoch bei Ausfall), **Gotify** (URL, App-Token),
  **Telegram** (Bot-Token, Chat-ID), **Webhook** (URL, JSON mit den Ereignissen, optional `Authorization: Bearer`).
- **Tabelle** `notification_channels` (Art, Name, aktiv, URL, Ziel, Token verschlüsselt mit eigenem Kontext je Spalte, „auch Warnungen“).
  Der Sendelog `notifications` bekommt eine Spalte für den Kanal (Mail oder Kanal-ID).
- **Versand:** Der `Notifier` sammelt wie bisher 10 s und schickt dieselbe Zusammenfassung an die Mail und an jeden aktiven Kanal. Der Text kommt aus
  `alerting::compose`, für Push als kurze Fassung (Titel plus wenige Zeilen, „+ 12 weitere“). Fehler eines Kanals blockieren die anderen nicht.
  Abhängigkeiten und Pausen gelten wie bei der Mail.
- **Oberfläche:** Seite „Benachrichtigungen“ (die Mail-Einstellungen werden ein Abschnitt darin), je Kanal „Testnachricht senden“, Sendelog mit Kanal.
  Nur Admins. Tokens erscheinen nur als „gesetzt/nicht gesetzt“.
- **Datenschutz:** In der Doku: Topics auf ntfy.sh sind für jeden lesbar, der den Namen kennt. Also langen Zufallsnamen oder Token nutzen, oder ntfy
  selbst betreiben. Die Nachricht enthält Geräte- und Sensornamen.
- **Tests:** Versand gegen einen lokalen HTTP-Testserver je Art, Kurzfassung, Fehler eines Kanals. Doku in `docs/notifications.md`.

Zu entscheiden:

1. Welche Kanäle? Empfehlung: ntfy und Webhook zuerst (decken fast alles ab, auch Home Assistant über dessen Webhook), Gotify und Telegram danach,
   weil sie nur kleine Ergänzungen sind.
2. ntfy.sh (fremder Server) oder selbst betreiben (z. B. als Docker-Container auf dem Server, in `Serveradmin/`)? Das betrifft nur den Betrieb,
   nicht den Code.

### G. Verfügbarkeit in Prozent und Berichte – umgesetzt 2026-10-27 (ohne Monatsmail)

**Umgesetzt** (`src/availability.rs`, `storage::availability`, `web::reports`, Migration `0021`, `docs/reports.md`): Zählung in `record_poll` je Stunde,
Lücken über drei Intervallen und pausierte Zeit zählen nicht, Rollup je UTC-Tag im Wartungslauf, Stunden nach `retention_hourly_days` gelöscht. Seite
`/reports` (alle Benutzer) je Gerät (Master, sonst Ping, sonst Mittel) oder Sensor, 7/30 Tage oder einer der letzten zwölf Monate in der Zeitzone des
Servers (Entscheidung 1 mit `jiff` aus J), sortierbar, CSV mit Schutz gegen Formeln; Kacheln 24 h/7/30 Tage auf der Sensorseite. Monatsmail später
(Entscheidung 2). Genauigkeit eine Stunde an den Rändern des Zeitraums.

Ziel: wie die Berichte in PRTG sehen, wie zuverlässig ein Gerät oder Sensor über 7 bzw. 30 Tage oder einen Monat war.

- **Befund:** Die Ereignistabelle `events` hält nur die neuesten 500 Wechsel und taugt dafür nicht. Daher eine neue Tabelle
  `availability_1h` (Sensor, Stunde, Millisekunden je Zustand In Ordnung/Warnung/Ausfall/Unbekannt/Pausiert) und ein Rollup `availability_1d`.
- **Zählen** in `storage::status::record_poll` in derselben Transaktion: die Zeit seit der letzten Abfrage geht an den vorigen Zustand, auf die
  Stunden verteilt. Lücken über dem Dreifachen des Intervalls (Dienst gestoppt, Rechner aus) zählen als „keine Daten“, nicht als verfügbar.
  Aufbewahrung wie die Messwerte: Stunden für `retention_hourly_days`, Tage unbegrenzt (bei 70 Sensoren etwa 25 000 Zeilen im Jahr).
- **Rechnung wie in PRTG:** verfügbar = In Ordnung + Warnung, nicht verfügbar = Ausfall. Pausierte Zeit (von Hand, durch Abhängigkeit, später
  Wartungsfenster) und Zeit ohne Daten zählen nicht mit. Ein Gerät gilt als verfügbar nach seinem Master-Sensor, sonst nach dem Ping-Sensor, sonst
  als Mittel seiner Sensoren.
- **Anzeige:** Sensordetail mit 24 h, 7 Tagen, 30 Tagen und Ausfallzeit. Neue Seite „Berichte“ (`/reports`, auch für Leser): Tabelle je Gerät bzw.
  Sensor für einen wählbaren Zeitraum (7 Tage, 30 Tage, ein Monat), sortierbar wie die Statuslisten, Download als CSV.
- **Monatsbericht per Mail** (optional, Schalter in den Benachrichtigungen): am Ersten eines Monats die Verfügbarkeit des Vormonats je Gerät, die
  schlechtesten zuerst.
- Verfügbarkeit gibt es erst ab dem Update, nachträglich lässt sie sich nicht berechnen.
- **Tests:** Zählung über Stundengrenzen, Lücken, Pausen, Rollup, Rechnung für Geräte, Handler. Doku neu `docs/reports.md`.

Zu entscheiden:

1. Monatsgrenzen in welcher Zeitzone? Gespeichert wird in UTC. Empfehlung: in der Zeitzone des Servers (braucht dieselbe Zeitzonenbibliothek wie J;
   ohne J zunächst UTC, dann liegen die Grenzen 1–2 h verschoben).
2. Monatsbericht per Mail gleich mit oder später? Empfehlung: später, zuerst Seite und CSV.

### H. `cargo audit` in der CI – umgesetzt 2026-10-27

**Umgesetzt** wie geplant (Job `audit`, montags 05:17 UTC und von Hand, `cargo-audit` 0.22.2 mit Cache; `dev/release.sh` bricht ohne `cargo-audit` oder bei Meldungen ab).

Befund aus dem Sicherheitstest: Dependabot hatte die vier RustSec-Meldungen nicht gemeldet, `cargo audit` schon.

- Neuer Job `audit` in `.github/workflows/ci.yml`: bei Push und Pull Request und zusätzlich **wöchentlich per `schedule`** (Montag früh) sowie von Hand
  (`workflow_dispatch`). So fallen neue Meldungen auf, auch wenn gerade nicht entwickelt wird. GitHub schickt bei einem Fehlschlag eine Mail an den
  Besitzer.
- Weil Actions nur von `actions/*` kommen dürfen: `cargo install cargo-audit --locked --version <fest>` mit `actions/cache` auf das Binary
  (Schlüssel = Version), die Datenbank holt `cargo audit` selbst. Ausnahmen bleiben in `.cargo/audit.toml` mit Begründung.
- Schwachstellen lassen den Job fehlschlagen, Warnungen (unmaintained, yanked) nur anzeigen; Letzteres prüft der Release.
- `dev/release.sh` prüft `cargo audit` ebenfalls vor dem Veröffentlichen.
- Doku: Abschnitt „Tests“/„Befehle“ in CLAUDE.md, `CONTRIBUTING.md`, `SECURITY.md`.

Zu entscheiden: nichts Grundsätzliches. Empfehlung: Wochenlauf ja.

### I. Hinweis auf neue Versionen in der Oberfläche – umgesetzt 2026-10-27

**Umgesetzt** (`src/update_check.rs`, `docs/operations.md`, „Notice of new versions“): Config `update_check` (Code-Standard aus, ausgelieferte Config an),
erste Abfrage fünf Minuten nach dem Start, dann alle 24 h; Ergebnis im `AppState` (`latest_release`), Hinweis über der Versionsnummer nur für Admins.
Die API-Adresse kommt aus `repository` in `Cargo.toml`; der Link muss auf die Releases dieses Repos zeigen. Gegen die echte API geprüft (als 0.4.4:
„0.5.0 verfügbar“, als 0.5.0: nichts). Abweichungen vom Plan: kein `If-None-Match` (eine Abfrage am Tag liegt weit unter dem Limit, der HTTP-Client
kennt keine eigenen Header), keine Anzeige, wann zuletzt geprüft wurde (Fehler stehen im Log auf `debug`).

Offener Punkt „Update-Möglichkeit“, erster Schritt. Installiert wird weiterhin von Hand (`apt install ./….deb`), weil das root braucht und die
Unit bewusst ohne Rechte läuft.

- Hintergrundaufgabe einmal am Tag (erste Abfrage einige Minuten nach dem Start): `GET https://api.github.com/repos/tomCSLK/opennetmon/releases/latest`
  über den HTTP-Client aus E, mit `User-Agent` und `If-None-Match`. Vorabversionen zählen nicht. Ergebnis nur im Speicher.
- Ist die Version neuer als `web::VERSION`, zeigt die Seitenleiste **für Admins** „Version 0.x.y verfügbar“ mit Link auf die Release-Seite. Auf der
  Seite „Über“ bzw. unter der Versionsnummer steht, wann zuletzt geprüft wurde und mit welchem Ergebnis.
- Config `update_check = true|false`. Fehler (kein Internet, Rate-Limit) nur im Log auf Stufe `debug`, nie als Störung in der Oberfläche.
- Datenschutz in `docs/operations.md`: GitHub sieht einmal am Tag die Adresse des Servers, sonst wird nichts gesendet.
- Tests: Versionsvergleich, Antwort eines lokalen Testservers, Anzeige nur für Admins.
- Später möglich (nicht Teil davon): eigenes apt-Repository auf GitHub Pages, dann reicht `apt upgrade`.

Zu entscheiden: Standard an oder aus? Empfehlung: in der ausgelieferten Config **an** mit Kommentar, im Code-Standard aus.

### J. Wartungsfenster nach Zeitplan – umgesetzt 2026-10-27

**Umgesetzt** (`src/schedule.rs`, `storage::schedules`, `web::schedules`, Migration `0020`; Beschreibung in `docs/devices-and-groups.md`,
„Maintenance windows“): Zeitpläne `/schedules` mit wöchentlichen und einmaligen Fenstern, Zuordnung `/maintenance/<kind>/<id>` (übernehmen, Zeitplan,
keiner), Wirkung über `Source::Schedule` in `pause::Pauses` (Pause von Hand geht vor, Abhängige folgen), Protokoll `schedule_log` (ein Eintrag je Fenster),
Zeitzone mit `jiff` 0.2 (Entscheidung 1; MIT/Unlicense, unter Linux nur `jiff` und `jiff-core`), Config `time_zone`. Abweichungen: Zuordnungen in einer
eigenen Tabelle `schedule_assignments` statt Spalten (kein Neuaufbau von `sensors`); „nächstes Wartungsfenster“ steht auf der Zuordnungsseite und in der
Zeitplanliste, nicht auf jeder Gruppen-, Geräte- und Sensorseite; Gerätevorlagen übernehmen keine Zeitpläne; Zeiten auf den Zeitplanseiten in der Zeitzone
des Servers (sonst überall die des Browsers).

Ziel: geplante Unterbrechungen (z. B. nächtliches Omada-Backup, Neustarts, Updates am Server) ohne Ausfallmails, ohne jedes Mal von Hand zu pausieren.

- **Zeitpläne** wie in PRTG als eigene, benannte Objekte (Seite `/schedules`, nur Admins): wiederkehrend (Wochentage, Beginn, Dauer, z. B.
  „täglich 03:00, 15 min“) oder einmalig (Beginn und Ende, z. B. „Server-Update am Samstag 10–11 Uhr“). Tabelle `schedules` plus Einträge.
- **Zuordnung** je Gruppe, Gerät und Sensor wie bei der Abhängigkeit: übernehmen (Standard), bestimmter Zeitplan, keiner. Vererbung nach unten.
- **Wirkung** wie eine Pause: berechnet in `pause::Pauses` als neue Quelle `Source::Schedule` (nicht gespeichert), Zustand „Pausiert“ mit dem Namen
  des Zeitplans, keine Abfragen, keine Mails. Danach weiter wie nach einer Pause von Hand (keine nachträgliche Erholungsmail). Das Protokoll zeigt
  Beginn und Ende je Fenster (ein Eintrag je Fenster, nicht je Sensor). Abhängige Objekte folgen über die Abhängigkeit wie bei einer Pause.
- **Zeitzone:** Zeitpläne gelten in Ortszeit, sonst verschieben sie sich bei der Sommerzeit. Dafür ein Crate mit Zeitzonen (Entscheidung 1), Zone vom
  System (`/etc/localtime`), überschreibbar per Config `time_zone = "Europe/Berlin"`.
- **Prüfung** im bestehenden 15-s-Takt des Pollers (`storage::pauses::sync_all`). Genauigkeit also etwa 15 s.
- Anzeige: auf Gruppen-, Geräte- und Sensorseite „nächstes Wartungsfenster: …“; Knopf im Menü „⋯ Aktionen“ (Einrichtung).
- **Tests:** Berechnung der Fenster (auch über Mitternacht und an den Tagen der Zeitumstellung), Vererbung, Poller ohne Mail, Handler. Doku in
  `docs/devices-and-groups.md` („Maintenance windows“).

Zu entscheiden:

1. Zeitzonen-Crate: `jiff` (MIT/Unlicense, liest die Zeitzonendatenbank des Systems, aktiv gepflegt) oder `chrono` plus `chrono-tz` (bringt eine
   eigene Datenbank mit, größer). Empfehlung: `jiff`.
2. Benannte Zeitpläne wie in PRTG oder Fenster direkt am Objekt? Empfehlung: benannt, weil man dasselbe Fenster an mehrere Geräte hängt.

### K. Zwei-Faktor-Anmeldung (TOTP)

Von außen schützt Cloudflare Access mit MFA, im LAN reicht über Port 8443 das Passwort.

- **Je Benutzer freiwillig**, eingeschaltet unter „Mein Konto“: Secret erzeugen, als QR-Code und als Text zeigen, mit einem Code bestätigen. Dazu
  zehn **Wiederherstellungscodes** (einmal angezeigt, gespeichert nur als SHA-256, je einmal nutzbar).
- **Anmeldung in zwei Schritten:** nach richtigem Passwort keine Sitzung, sondern ein kurzlebiger Zwischenschritt (zufälliges Token, 5 min, nur als
  Hash gespeichert) und die Seite „Code eingeben“. Erst ein gültiger Code (30-s-Schritte, ±1 Schritt Toleranz) erzeugt die Sitzung. Derselbe Code gilt
  nur einmal (`users.totp_last_step`). Fehlversuche zählen in `auth::throttle` wie falsche Passwörter.
- **Umsetzung:** TOTP nach RFC 6238 selbst (wenige Zeilen; `hmac` und `sha1` werden über `snmp2` schon gebaut, kommen nur als direkte Abhängigkeit
  dazu), Base32 selbst. Secret verschlüsselt mit `SecretBox` (Kontext `users.totp_secret`). Für den QR-Code ein Crate (Entscheidung 1).
- **Zurücksetzen:** Admins auf `/users/<id>` (beendet die Sitzungen des Benutzers), dazu CLI `opennetmon user 2fa-reset <name>`, falls der einzige
  Admin sein Handy verliert. „Mein Konto“ zeigt, ob 2FA an ist; Ausschalten nur mit Passwort und Code.
- „Angemeldet bleiben“ (`stay_signed_in`) und das Abmelden ohne Aktivität bleiben unverändert.
- **Tests:** Codes aus den Testvektoren von RFC 6238, Ablauf des Zwischenschritts, Wiederverwendung, Wiederherstellungscodes, Throttle, CSRF,
  Handler. Doku in `docs/users-and-login.md`.

Zu entscheiden:

1. QR-Code: Crate `qrcode` (MIT/Apache-2.0, SVG-Ausgabe ohne weitere Abhängigkeiten) oder nur den Text zum Abtippen. Empfehlung: `qrcode`.
2. Pflicht für Admins einstellbar (Config `require_2fa_for_admins`)? Empfehlung: ja, aber Standard aus.

### L. Rotation von `secret.key`

Offener Punkt. Heute lässt sich der Schlüssel nicht wechseln, ohne alle SNMP-Passwörter und das SMTP-Passwort neu einzugeben.

- **CLI** `opennetmon key rotate` bei gestopptem Dienst:
  1. Sicherung der Datenbank (`VACUUM INTO`, wie `opennetmon backup`)
  2. neuen Schlüssel als `secret.key.new` (0600) schreiben
  3. in einer Transaktion alle verschlüsselten Werte mit dem alten Schlüssel entschlüsseln und mit dem neuen verschlüsseln
  4. Dateien umbenennen: `secret.key` → `secret.key.old-<Datum>`, `secret.key.new` → `secret.key`
- **Verzeichnis der verschlüsselten Spalten** an einer Stelle (Tabelle, Spalte, Kontext). Ein Test prüft, dass jede Spalte `*_enc` im Schema darin
  steht, damit spätere Secrets (F, K) nicht vergessen werden.
- **Schlüssel-Kennung:** neue Tabelle `meta` mit dem Fingerabdruck des Schlüssels (gekürzter SHA-256), in derselben Transaktion geschrieben. Der Server
  prüft beim Start, ob Schlüssel und Datenbank zusammenpassen, und meldet sonst klar „Schlüssel passt nicht zur Datenbank“. Das hilft auch heute
  schon, wenn eine Sicherung mit dem falschen Schlüssel zurückgespielt wird; bricht der Ablauf zwischen Schritt 3 und 4 ab, sagt die Meldung, welche
  Datei passt.
- **Dienst gestoppt:** Der Server hält beim Laufen eine Sperre auf eine Datei neben der Datenbank (`flock` über `rustix`), `key rotate` bricht ab,
  wenn sie belegt ist. Sonst würde der laufende Server mit dem alten Schlüssel weiterschreiben.
- Doku in `docs/operations.md`: Sicherungen von vorher brauchen den alten Schlüssel, ihn also bei den Sicherungen aufbewahren.
- **Tests:** Rotation mit Profilen und Mail-Passwort, Abbruch mittendrin, falscher Schlüssel beim Start, Sperre.

Zu entscheiden: nichts Grundsätzliches. Empfehlung: die Startprüfung des Fingerabdrucks gleich mit einbauen.

### M. Änderungsprotokoll (wer hat wann was geändert) – umgesetzt 2026-10-27

**Umgesetzt** (`src/audit.rs`, `storage::audit`, `web::audit`, Migration `0022`, `docs/users-and-login.md`, „Audit log“): Seite `/audit` („Änderungen“,
nur Admins) mit Filterfeld, Filter je Benutzer über den Namen und Link aufs Objekt; erfasst alle Admin-Handler (alt → neu, Entscheidung 1; gleich
gespeicherte Formulare ohne Eintrag) und An-/Abmeldungen samt fehlgeschlagener mit Adresse (Entscheidung 2). Abweichungen: Einträge **nach** der
Änderung statt in derselben Transaktion (ein Fehler beim Schreiben wird geloggt, die Änderung bleibt); bei fehlgeschlagenen Anmeldungen nur der
Name eines vorhandenen Kontos (ein ins Namensfeld getipptes Passwort käme sonst ins Protokoll); Filter nach Objekt nur über das Suchfeld, keine
Sortierung per Spaltenkopf (neueste zuerst); höchstens 2000 Einträge auf der Seite. Aufbewahrung `retention_audit_days` (Standard 365).

- **Tabelle** `audit_log` (Zeit, Benutzer, Adresse, Aktion, Objektart, Objekt-ID, Objektname zum Zeitpunkt, Kurztext als `Msg::encode`).
- **Erfasst:** Anlegen, Ändern und Löschen von Gruppen, Geräten, Sensoren, SNMP-Profilen, Benutzern, Vorlagen und MIB-Dateien, dazu Schwellwerte,
  Abhängigkeiten, Benachrichtigungen, Reihenfolge, An- und Abmelden (auch fehlgeschlagene Anmeldungen) und später 2FA (K). Pausen stehen schon im
  Protokoll.
- **Nie** Secrets: bei Passwörtern und Tokens nur „geändert“. Bei anderen Feldern alt → neu (Entscheidung 1).
- Geschrieben in den Admin-Handlern über einen kleinen Helfer (`audit::record`), in derselben Transaktion wie die Änderung, wo es eine gibt.
- **Seite** `/audit` nur für Admins: sortierbar und durchsuchbar wie `/log`, Filter nach Benutzer und Objekt, Link aufs Objekt.
- Aufbewahrung 365 Tage (Config `retention_audit_days`), vom bestehenden Wartungslauf gelöscht.
- **Tests:** je Handler-Gruppe ein Eintrag, keine Secrets im Protokoll (Test über alle Einträge eines Durchlaufs), Rollen. Doku in
  `docs/users-and-login.md`.

Zu entscheiden:

1. Nur „Feld X geändert“ oder alt → neu? Empfehlung: alt → neu (außer Secrets), sonst ist das Protokoll wenig wert.
2. Anmeldungen mitprotokollieren? Empfehlung: ja, auch fehlgeschlagene (mit Adresse); das journald-Log hat sie heute nur auf Stufe `info`.

### Fehlgeschlagene Abfragen in der Messwerttabelle – umgesetzt 2026-10-27 (Wunsch des Nutzers, nach einem Bildschirmfoto von PRTG)

Vorher fehlten fehlgeschlagene Abfragen in der Tabelle „Messwerte“ ganz (sie schreiben keine Messwerte). Jetzt speichert `record_poll` jede Abfrage,
die nicht „OK“ endet, in `poll_problems` (Migration `0023`: Status und Meldung, Aufbewahrung wie Rohwerte); `recent_samples` mischt sie als Zeilen ein
(`TableRows`), die Tabelle färbt sie rot bzw. gelb und zeigt bei fehlenden Werten die Meldung. Mittelwert, Minimum und Maximum zählen nur Zeilen mit Werten.
Nicht übernommen aus PRTG: Spalten „Ausfallzeit“ und „Abdeckung“ (PRTG zeigt sie in den gemittelten historischen Daten; unsere Tabelle hat je Abfrage
eine Zeile, dort wären es nur 0 oder 100 %).

### Sensor bearbeiten – umgesetzt 2026-10-27 (Wunsch des Nutzers)

Seite „Einstellungen“ (`/sensors/<id>/edit`, nur Admins, im Menü „⋯ Aktionen“ der Sensorseite) für Sensoren mit eigenen Einstellungen: eigene OID (OID,
Auswertung, Faktor, Einheit) und Dienste (HTTP(S), TCP, DNS, TLS). Dieselben Felder wie „Sensor hinzufügen“ (`templates/sensor_fields.html`),
`storage::sensors::update_spec` (Typ bleibt, Name/Grenzwerte/Verlauf bleiben, bei Wechsel Wert ↔ Zähler die Einheit des Kanals), Änderungsprotokoll je Feld.
Ping, Uptime, Interfaces und „Dieser Server“ haben keine Einstellungen (Name und Intervall stehen schon auf der Sensorseite).

### Kleinere offene Punkte (ohne eigenen Plan)

- Test mit dem echten Mailserver des Nutzers (braucht den Nutzer), Ende-zu-Ende-Integrationstest (Server starten, Gerät gegen lokalen `snmpd`, Poller,
  Statuswechsel, Mail an einen Test-SMTP-Server).
- Gerätevorlagen übernehmen das Intervall je Sensor und die Abhängigkeitseinstellungen außer dem Master noch nicht.
- ER605 zusätzlich über die Omada-API (nicht im MVP, nur auf Wunsch).

## Bewusst nicht im MVP

SNMP-Traps, NetFlow/sFlow, verteilte Probes, Netzwerkkarten/Maps, SNMP SET, Mandantenfähigkeit, Omada-Controller-API,
Herstellermibs außer gezielt benötigten TP-Link-OIDs.
Zurückgestellt gegenüber früheren Ideen: SNMP v1, Subnetz-Scan, Docker-Betrieb, htmx.
Verworfen am 2026-10-27 nach Abwägung: RPC über UDP (als Sensor für ONC-RPC reicht bei NFSv4 der TCP-Port-Sensor aus E; als Schnittstelle
von opennetmon wäre eine JSON-API über HTTPS der bessere Weg).

Optionen, erst auf Nachfrage (z. B. ein GitHub-Issue):
- Knopf „Protokoll“ auch auf Gruppenseiten (`/log?group=<id>` mit allen Geräten der Gruppe und ihrer Untergruppen); am 2026-10-24 bewusst
  nicht umgesetzt, weil das Protokoll großer Gruppen wieder unübersichtlich würde. Geräte- und Sensorseiten haben den Knopf seit v0.4.4.

## Verifikation

- **Unit- und Handler-Tests** (544, `cargo test`): Counter-/Ratenberechnung (inkl. Wrap/Reset), Statuslogik, Pausen, Abhängigkeiten, Config, Auth, Krypto, Validierung,
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
- Schlüsseldatei (`secret.key`): Sicherung getrennt von der Datenbank einplanen; Schlüsselrotation ist noch nicht umgesetzt (geplant als Vorhaben L)
- Update-Möglichkeit: Weg und Zeitpunkt (erster Schritt geplant als Vorhaben I)

Geklärt: Beiträge nach DCO (2026-10-21). Der Server hat Internetzugang und betreibt weitere Programme (2026-10-21).
