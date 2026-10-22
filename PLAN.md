# Plan: opennetmon – SNMP-Monitor für Linux

> Dieser Plan ergänzt `CLAUDE.md`. Bei Widersprüchen gilt `CLAUDE.md`.
> Der Name ist **opennetmon** (festgelegt am 2026-10-19 mit neuem Logo; vom 2026-10-16 bis dahin „netmon“). „PRTG" ist eine Marke von Paessler; opennetmon ist nur vom Bedienkonzept inspiriert.

## Stand (2026-10-21, Release v0.4.1)

Alle sechs MVP-Schritte sind umgesetzt, dazu einiges darüber hinaus. Als Nächstes: **Installation auf dem echten Server**,
Test mit dem **echten Mailserver** des Nutzers, ein Ende-zu-Ende-Integrationstest und Feinschliff. Geplant ist außerdem eine
**Update-Möglichkeit** (Zeitpunkt offen).

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
  „Aktualisieren“ mit automatischem Neuladen alle 60 s
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

Releases (alle Vorabversionen, x86_64): **v0.1.0** (2026-10-19), **v0.2.0** (2026-10-20, Mehrsprachigkeit, .deb), **v0.3.0** (2026-10-20, eingebautes HTTPS),
**v0.3.1** (2026-10-20, Statuslisten hinter den Kacheln), **v0.4.0** (2026-10-21, Pausieren, Protokoll, Gerätebaum mit Gerätetypen),
**v0.4.1** (2026-10-21, Sensoren „Dieser Server“).

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

Migrations `0001`–`0016` (dazu einmalige Neuaufbauten von `devices` und `sensors` in `storage::migrate`): `meta`, `users`, `sessions`, `credential_profiles` (Secrets verschlüsselt), `groups` (hierarchisch, höchstens 8 Ebenen),
`devices` (Profil optional für Geräte ohne SNMP, Gerätetyp `kind`), `sensors`, `channels` (Einheit, Schwellwerte), `value_states` (Bewertung von
Aufzählungswerten), `samples`, `samples_1h`, `samples_1d`, `sensor_status`, `events`, `mail_settings`, `notifications`, `mib_files`,
`device_templates`, `pause_log`; `sensors.target` für die Sensoren „Dieser Server“; Pausen als Spalten an `groups`/`devices` (`paused`, `pause_until_ms`, `pause_reason`), Spalte `position` für die Handordnung.

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
13. **Offen:**
    - Installation auf dem echten Ubuntu Server (Paket), `/healthz` und Login über HTTPS prüfen, die sieben Omada-Geräte einrichten
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

## Bewusst nicht im MVP

SNMP-Traps, NetFlow/sFlow, verteilte Probes, Netzwerkkarten/Maps, SNMP SET, Mandantenfähigkeit, Omada-Controller-API,
Herstellermibs außer gezielt benötigten TP-Link-OIDs.
Zurückgestellt gegenüber früheren Ideen: SNMP v1, Subnetz-Scan, Docker-Betrieb, htmx.

## Verifikation

- **Unit- und Handler-Tests** (436, `cargo test`): Counter-/Ratenberechnung (inkl. Wrap/Reset), Statuslogik, Pausen, Config, Auth, Krypto, Validierung,
  Storage, SNMP-Client gegen den Fake-Agenten, MIB-Parser, Übersetzungskataloge; Handler mit `tower::ServiceExt::oneshot` (Helfer in `web/testutil.rs`).
- **Integrationstests** gegen den lokalen `snmpd` (`tests/snmp_local.rs`, 5 Tests: Probe, falsche Community, v3 mit richtigem und falschem Passwort,
  64-Bit-Counter); der Ende-zu-Ende-Test steht noch aus (siehe „Offen“).
- **Gerätetests** gegen echte Omada-Hardware nur manuell (`tests/omada_manual.rs`, `#[ignore]`).
- **Oberfläche:** Änderungen im Browser mit einer Wegwerfkopie der Entwicklungsdaten prüfen, hell und dunkel.
- **Packaging:** `dev/release.sh` prüft Archiv, Installer-Probelauf und Inhalt des .deb; Paket im Podman-Container getestet (Installation, Update, remove, purge).
- **Vor jedem Abschluss:** `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.

## Sicherheit (Kurzfassung)

SNMP nur lesend; Secrets verschlüsselt in der DB (Schlüsseldatei 0600, Rechteprüfung beim Start), nie in Logs oder HTML; argon2;
`HttpOnly`/`SameSite=Strict`-Cookies, `Secure` per Config; CSRF-Schutz für alle POST-Formulare; serverseitige Validierung
von IP, OID, Intervallen; nur HTTPS (eingebaut, Klartext-HTTP nur Weiterleitung) oder Bind auf `127.0.0.1` hinter einem TLS-Proxy;
Begrenzung der Anmeldeversuche (`auth::throttle`).

## Offene Punkte (vom Nutzer zu klären)

- ER605: läuft als Gerät ohne SNMP (nur Ping); später zusätzlich über die Omada-API?
- Schlüsseldatei (`secret.key`): Sicherung getrennt von der Datenbank einplanen; Schlüsselrotation ist noch nicht umgesetzt
- Repo öffentlich machen: vorbereitet (2026-10-21), Umschalten durch den Nutzer
- Update-Möglichkeit: Weg und Zeitpunkt

Geklärt: Beiträge nach DCO (2026-10-21). Der Server hat Internetzugang und betreibt weitere Programme (2026-10-21).
