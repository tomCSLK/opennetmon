# CLAUDE.md – opennetmon

> Name: **opennetmon** („Network Monitoring“), festgelegt am 2026-10-19 (vorher seit 2026-10-16 „netmon“); Crate, Binary, Paket und GitHub-Repo (`tomCSLK/opennetmon`) heißen so.
> Logo: Original außerhalb des Repos (`~/Bilder/opennetmon_Logo_v1.jpeg`), daraus `static/logo-mark.png` (Seitenleiste, Touch-Icon),
> `static/favicon-32.png`/`-64.png`, `static/logo-wide.jpg` (Anmeldeseite) und `docs/assets/opennetmon-logo.jpg` (README).
> „PRTG" ist eine Marke von Paessler; opennetmon ist nur vom Bedienkonzept inspiriert und steht in keiner Verbindung zu Paessler.
> Umbenannt, weil „netmon“ markenrechtlich geschützt ist; für „opennetmon“ hat der Nutzer keine Registereinträge gefunden.
> Lizenz: **AGPL-3.0-or-later**, Copyright Thomas Cieslik (`LICENSE`, `Cargo.toml` `license`/`repository`/`authors`, README-Abschnitt „License“).

## Ziel

Agentloses Netzwerk-Monitoring per SNMP mit Web-Oberfläche, inspiriert vom Bedienkonzept von PRTG.
Läuft headless auf einem **Ubuntu Server ohne GUI**; Bedienung ausschließlich über den Browser.
Entwickelt wird auf Zorin OS (Desktop), betrieben auf Ubuntu Server.

Zielumgebung: privates Netz mit TP-Link Omada (Software-Controller): Gateway ER605, drei Smart-Switches
(2× SG2008, 1× SG2008P), drei Access Points (1× EAP225, 2× EAP653). SNMP wird im Controller site-weit konfiguriert
und ist dort nur als **SNMPv3 authNoPriv mit MD5** aktiv (v1/v2c aus, kein Privacy-Passwort). Gemessen im Spike:
Switches und Access Points antworten, der ER605 nicht. Switches liefern 64-Bit-Counter (`ifXTable`), Access Points nur
32-Bit-Counter (`ifTable`). Die Omada-Controller-API ist **nicht** Teil des SNMP-MVP.

## Sprachentscheidung: Rust (entschieden)

**Phase 0 (Spike) ist abgeschlossen, Entscheidung am 2026-10-15: Rust bleibt.** `examples/snmp_probe` fragt
`sysDescr`, `sysUpTime`, `ifHCInOctets` und GETBULK-Walks über `ifTable`/`ifXTable` per SNMPv2c und SNMPv3
(authNoPriv und authPriv) ab und lief gegen den lokalen `snmpd` (SHA-256/AES-128) sowie gegen alle SNMP-fähigen Omada-Geräte
(authNoPriv/MD5). Details und Messwerte: `docs/decisions/0001-language.md`.

Der Python-Fallback (unten) ist **nicht** gewählt und nur noch als Dokumentation der Alternative erhalten.
Die Sprache nicht eigenmächtig wechseln – bei grundlegenden Problemen Befund dokumentieren und nachfragen.

## Stack (Rust)

- Runtime/Web: `tokio`, `axum`, `tower-http` (Static, Compression, Trace)
- Templates/UI: `askama` (server-seitiges HTML), **uPlot** für Graphen, sonst nur wenige eigene Skripte in `static/`
  (kein htmx: am 2026-10-21 gestrichen, die Oberfläche kommt ohne aus).
  JS/CSS lokal vendoren (kein CDN – die Oberfläche soll ohne fremde Server funktionieren), per `rust-embed` ins Binary.
  Kein npm, kein WASM-Frontend.
- Design: `static/app.css` ist ein kleines Designsystem (Farb-Tokens für hell/dunkel, Seitenleiste, Kacheln, Karten, Badges, Tabellen, Formulare);
  Schema folgt dem System, Umschalter in der Seitenleiste (`static/theme.js` im `<head>` gegen Aufblitzen, `static/app.js` für Umschalter, aktiven Menüpunkt
  und Handy-Menü). Keine Inline-Styles (CSP): Breiten z. B. des Statusbands der Mini-Graphen als SVG-Attribute. Graphen lesen ihre Farben aus den CSS-Variablen
  und zeichnen bei Schemawechsel neu (Event `opennetmon:theme`).
  Handy (`docs/devices-and-groups.md`, „Phones“): Zeilenaktionen als Symbole aus den Makros in `templates/icons.html`; Knöpfe der Seitenköpfe im Menü
  `details.more-actions` (breit eine Knopfreihe, bis 640 px Dropdown über `static/app.js`; `theme.js` setzt die Klasse `js` an `<html>`); auf Geräte-, Gruppen-
  und Sensorseite steht in der Reihe nur Pausieren/Fortsetzen und Protokoll, die Einrichtung im inneren `details.more-actions.setup` (Dropdown auf jeder Breite,
  auf dem Handy Einträge des äußeren Menüs unter einer Linie; Sprunglinks wie `#delete` schließen das Menü; Test `page_heads_show_everyday_actions…`); Tabellen bis 640 px
  als Blöcke (`table.m-cards` + Layoutklasse wie `m-sensors`/`m-log`/`m-simple`, Zellklassen `m-name`, `m-status`, `m-value`, `m-hide` …)
- Statische Dateien (`web::assets`): `Cache-Control: no-cache` + ETag (SHA-256 des eingebetteten Inhalts), Antwort `304` bei unverändertem Stand –
  neue Versionen gelten nach einem Update sofort, ohne Versionsnummern in den Adressen pflegen zu müssen
- SNMP: `snmp2` mit Features `tokio`, `crypto-rust` (keine OpenSSL-Abhängigkeit) und **`heap_buffers`** (Pflicht: sonst liegt ein 64-KB-Puffer
  auf dem Stack und überläuft in Tokio-Threads mit 2 MB Stack)
- Zeitzonen: `jiff` 0.2 (Wartungsfenster in Ortszeit über Sommerzeitwechsel hinweg; Zone aus `/etc/localtime` bzw. Config `time_zone`, tzdb aus
  `/usr/share/zoneinfo`)
- Dateisysteme: `rustix` (Feature `fs`, sicheres `statvfs` ohne `unsafe`; war über tempfile schon im Lock) für die Sensoren „Dieser Server“
- Ping: `surge-ping` (0.9, ein Socket je Adressfamilie für alle Ziele); bevorzugt unprivilegierte ICMP-Sockets (`net.ipv4.ping_group_range`), sonst `CAP_NET_RAW`
- DB: SQLite über `sqlx` (Migrations in `migrations/`, WAL-Modus)
- Uploads: axum-Feature `multipart` (nutzt `multer`) für MIB-Dateien
- `serde_json` für die Regeln der Gerätevorlagen (war über axum schon im Build)
- Übersetzungen: `fluent-bundle` 0.16 (Fluent, Kataloge `locales/<code>/*.ftl` per `rust-embed`; nebenläufige Bundles, Isolationszeichen aus,
  Dezimalkomma per Formatter im deutschen Bundle) und `unic-langid` (Sprachkennung, war über fluent schon im Build)
- HTTPS: `tokio-rustls` 0.26 (`rustls` mit `ring`, ohne OpenSSL; war über lettre schon im Build), `hyper`/`hyper-util` (eigene Accept-Schleife mit
  `GracefulShutdown`, weil `axum::serve` kein TLS kann), `tower` (`map_request` für `ConnectInfo`), `rcgen` 0.14 (selbst signiertes Zertifikat, Features `crypto`,
  `pem`, `ring`) und `time` (Gültigkeit; kam mit rcgen)
- Mail: `lettre` 0.11 (Features `builder`, `smtp-transport`, `tokio1`, `tokio1-rustls-tls`; ohne native-tls/OpenSSL, damit der musl-Build statisch bleibt)
- HTTP-Client (`http_client`, für die Dienst-Sensoren, später Push und Update-Hinweis): `hyper` mit Feature `client` (eine Verbindung je Anfrage, HTTP/1.1),
  `http-body-util`, `tokio-rustls`, `webpki-roots` (Mozilla-Wurzeln; alle drei waren schon im Build); `x509-parser` 0.18 (ohne Default-Features) für Ablauf
  und Aussteller der Zertifikate
- Sonstiges: `serde`, `toml` (Config), `clap` (CLI), `tracing` + `tracing-subscriber`,
  `thiserror` (Lib-Fehler) / `anyhow` (nur in `main`), `argon2` (Passwörter, API ab 0.6: `hash_password`
  erzeugt den Salt selbst), `sha2` (Session-Token-Hash), `subtle` (Vergleich in konstanter Zeit), `rand` (Tokens),
  `rpassword` (Passwortabfrage in der CLI), `chacha20poly1305` (XChaCha20-Poly1305 für gespeicherte SNMP-Secrets; API ab 0.11:
  `XNonce::generate()`, `Key::from([u8; 32])`, AAD über `Payload`)
- Rust Edition 2024, stabile Toolchain
- `rust-embed` braucht das Feature `mime-guess`; `sqlx` das Feature `macros` für `migrate!` (keine Compile-Time-Queries)
- `snmp2` hat keinen eingebauten Timeout: jede Anfrage selbst mit `tokio::time::timeout` begrenzen

Neue Abhängigkeiten nur mit kurzer Begründung im Commit; keine unmaintained Crates; Lizenz muss mit AGPL-3.0 verträglich sein
(MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0 u. Ä.; nicht GPL-2.0-only, nicht proprietär). Nach jeder Änderung an `Cargo.lock`
`python3 -I dev/third-party-licenses.py` laufen lassen und `THIRD-PARTY-LICENSES.txt` mitcommitten.

## Stack (Python-Fallback, nicht gewählt)

FastAPI + Uvicorn, Jinja2 + htmx, `pysnmp` 7.x (asyncio-API – ältere synchrone Beispiele aus dem Netz
sind mit 7.x nicht kompatibel), SQLite via SQLAlchemy + Alembic, Paketverwaltung mit `uv`,
`ruff` + `mypy --strict` + `pytest`. Architektur und Datenmodell wie unten.

## Begriffe (Domänenmodell)

- **Gerät (Device):** Host mit IP/Hostname, SNMP-Zugangsdaten, Gruppe, Polling-Intervall
- **Sensor:** eine Messung an einem Gerät (z. B. „Traffic Port 1:24", „Uptime", „Ping")
- **Kanal (Channel):** ein Messwert eines Sensors (z. B. In-/Out-Bit/s), mit Einheit und Schwellwerten
- **Status:** `Up`, `Warning`, `Down`, `Unknown`, `Paused` – Sensorstatus aus Kanälen, Gerätestatus aus Sensoren
- **Gruppe:** hierarchische Ordnung der Geräte (Standort/Funktion)

## MVP-Umfang

1. Geräteverwaltung (CRUD), Gruppen, SNMP-Credentials (v2c, v3 authNoPriv/authPriv) als wiederverwendbare Profile – **umgesetzt**
   (`docs/snmp-profiles.md`, `docs/devices-and-groups.md`), inklusive Verbindungstest je Gerät
2. Sensortypen: Ping, SNMP Uptime, Interface-Traffic (IF-MIB, 64-Bit-HC-Counter), Interface-Status
   (`ifOperStatus`), benutzerdefinierter OID-Sensor (Gauge/Counter)
3. Auto-Discovery der Interfaces per Walk; Nutzer wählt, welche als Sensoren angelegt werden – **umgesetzt** (`docs/sensors.md`);
   darüber hinaus MIB-Import und „Werte suchen“ (PoE, CPU, Speicher, Hardware-Sensoren; neue Sensoren inaktiv) – **umgesetzt** (`docs/mibs.md`)
4. Schwellwerte je Kanal → Statusberechnung; Statusübersicht und Detailseite mit Graph (1h/24h/7d/30d) – **umgesetzt** (`docs/charts-and-thresholds.md`)
5. Login (lokale Benutzer), Rolle Admin/Leser – **umgesetzt** (erster Admin per CLI, danach Benutzerverwaltung in der Oberfläche `web::users`, siehe `docs/users-and-login.md`)
6. Benachrichtigung per E-Mail (SMTP) bei Statuswechsel – **umgesetzt**, Einstellungen in der Oberfläche (`docs/notifications.md`)

**Nicht im MVP:** SNMP-Traps, NetFlow/sFlow, verteilte Probes, Netzwerkkarten/Maps, SNMP SET,
Mandantenfähigkeit, Omada-Controller-API, Herstellermibs außer gezielt benötigten TP-Link-OIDs.

## Architektur

Ein Binary, ein Prozess, Module:
- `config` – TOML-Datei + ENV-Overrides (`OPENNETMON_LISTEN`, `OPENNETMON_DATABASE_PATH`, `OPENNETMON_LOG_FILTER`, `OPENNETMON_SECRET_KEY_FILE`); `poller_enabled`, `poll_concurrency`
- `i18n` – Sprachen (`Lang`: `code`, `native_name`, `from_accept_language`, `get`/`get1`/`get2`/`fmt`/`msg`), `Msg` (Katalog-ID mit Variablen oder
  `Msg::Text` unverändert; `Display` = Deutsch für Logs), Sprache der Anfrage als Task-Local (`scope`, `current`, `set_current`; gesetzt von der
  Middleware `web::request_language` aus `Accept-Language` und vom `CurrentUser`-Extractor aus `users.language`); gespeicherte Meldungen als
  `Msg::encode` (Zustand, Ereignisse, Sendelog); `js_texts()` liefert die `js-…`-Nachrichten als JSON-Datenblock für `static/*.js`
  (`T('key')` aus `static/theme.js`, Platzhalter `%1`); Tests prüfen die Kataloge gegeneinander und gegen Code, Templates und JS (`docs/translations.md`)
- `tls` – eingebautes HTTPS: `generate` (ECDSA P-256, zehn Jahre, SAN aus `certificate_names`: `--name`-Werte, Hostname, localhost, Loopback; ungültige Namen
  vom System überspringt die CLI), `load` (PEM-Kette + Schlüssel PKCS#8/PKCS#1/SEC1, Schlüssel 0600 Pflicht, Fingerabdruck), `serve` (ein Port, nur HTTPS: erstes Byte
  `0x16` → TLS, sonst `301` auf `https://` mit Host/Pfad aus der Anfrage bzw. `400`; Handshake- und Header-Timeouts); Config `tls_cert_file`/`tls_key_file`
  (beide oder keins), CLI `opennetmon tls generate [--name …] [--cert] [--key]`; ausgelieferte Config: `0.0.0.0:8443` mit HTTPS, Code-Standard ohne TLS `127.0.0.1:8080`
- `auth` – Rollen, Argon2-Hashing, Session-/CSRF-Tokens, Validierung von Benutzername und Passwort
- `snmp` – `SnmpClient` (GET und GETBULK-`walk` mit Timeout/Retries, v2c und v3, `heap_buffers`), `probe` für den Verbindungstest; `discovery` – Interface-Discovery über IF-MIB-Walks (`merge` ist reine Logik); Poller und Sensoren bauen darauf auf
- `mib` – MIB-Parser (`mib::lexer`, `mib::parser`, tolerant) und Index (`MibIndex`: Auflösung über Module, Textual Conventions, Tabellen inkl. AUGMENTS,
  Suche, Name ↔ OID, `oid_target` für das Sensorformular); mitgelieferte Standard-MIBs in `mibs/` (per `rust-embed`), Uploads in `mib_files`;
  OIDs höchstens 128 Stellen (`MAX_OID_ARCS`, begrenzt Rekursion und Speicher von `Resolver::resolve_def`; abgebrochene Auflösungen nicht im Memo; vorher
  brachte eine Kette von Definitionen den Server auf 12 GB und nach dem Speichern bei jedem Start zum Absturz);
  Index im `AppState` (`mib_index()`, `reload_mibs()`), Seite `web::mibs` (hochgeladene Dateien nach Hersteller gruppiert über `mib::vendors` (Unternehmensnummer der meisten Objekte, sonst die der importierenden Module; Namen für ~50 Hersteller), darin Probleme zuerst, Auswahl `?files=problems|ok`, Filterfeld `static/table-filter.js`, Tabelle mit fester Höhe `.table-wrap.scroll`), Anzeige-Helfer `web::sensor_text` (Namen, Klartext, Einheiten) (`docs/mibs.md`); `mib::catalog` – bekannte Tabellen (PoE, CPU, Speicher, HOST-RESOURCES, ENTITY-SENSOR), Einheiten aus Beschreibungen
- `ordering` – Anzeigereihenfolge: `natural_cmp` (Zahlen der Größe nach), `position_cmp` (Handordnung zuerst), `reorder` aus Formularpositionen;
  Spalte `position` in `groups`/`devices`/`sensors` (`NULL` = automatisch, beim Verschieben in eine andere Gruppe zurückgesetzt), Schreiben über
  `storage::write_positions`; Seiten `web::order` (`/groups/<id>/order`, 0 = oberste Ebene; `/devices/<id>/sensors/order`) mit `static/order.js`;
  Sensoren automatisch nach Typ, dann Name; Umbenennen von Sensoren auf der Detailseite (`sensor_detail::rename`)
- `templates` – Gerätevorlagen: `Body` (Regeln als JSON in `device_templates`, Migration `0011`), `build` aus den Sensoren eines Geräts,
  `plan` (reine Logik: nur Fehlendes, Übersprungenes mit Grund), `apply` (Interfaces lesen, OIDs per GET prüfen, Sensoren mit Schwellwerten anlegen),
  `detect` über `sysObjectID`; `storage::templates`, `devices.template_id`; Seiten `web::templates` (`/templates`, `/devices/<id>/template/new|apply`),
  Auswahl „automatisch erkennen“ beim Anlegen eines Geräts (`docs/device-templates.md`)
- `value_discovery` – Tabellen/Gruppen walken und Sensorvorschläge bilden (Namen, Faktor, Einheit); Seite `web::sensor_values`, neue Sensoren standardmäßig **pausiert** (`enabled = false`, Status „Pausiert“), Sammelaktionen `web::sensors::bulk`
- `pause` – Pausieren wie in PRTG: `Target` (Gruppe/Gerät/Sensor), `Pause` (Ende, Grund), `Pauses` (Vererbung Sensor → Gerät → Gruppen, `Why`/`Source`),
  `parse_duration`/`validate_reason`; `storage::pauses` (`load`, `pause`, `resume`, `expire` vom Poller alle 15 s, `log_entries`; Tabelle `pause_log`, neueste 500,
  Migration `0016`: `paused`/`pause_until_ms`/`pause_reason` an `groups`/`devices`, eigene Sensorpause bleibt `enabled = 0`); Seiten `web::pause`
  (`/pause/<kind>/<id>`, `/resume/<kind>/<id>`); beim Pausieren Zustand sofort „Pausiert“ (`storage::status::mark_paused`), erste Abfrage danach ohne alten Zustand
  (keine nachträgliche Erholungsmail); im Protokoll mit Problemen gemischt (`docs/devices-and-groups.md`, „Pausieren“)
- Abhängigkeiten wie in PRTG (2026-10-27, `docs/devices-and-groups.md`, „Dependencies“): je Gruppe/Gerät/Sensor `Dependency` (`parent` = übernehmen,
  `sensor` = von einem Sensor abhängig, `none`) und Verzögerung nach der Rückkehr (`Setting`, `parse_delay`, 0–3600 s, vererbt, Standard 0), dazu
  Master-Sensor je Gerät (`sensors.dependency_master`, pausiert die übrigen Sensoren seines Geräts); Migration `0019` (Spalten an `groups`/`devices`/`sensors`,
  Tabelle `dependency_log`). Berechnet in `pause::Pauses` (nicht als Pause gespeichert): `Source::Dependency`, ausgelöst bei „Ausfall“ oder „Pausiert“ des
  Sensors (auch von Hand; Ketten mit Zyklusschutz) und im Verzögerungsfenster; `dependency_unsettled` (Poller verwirft Fehlschläge, solange der Sensor,
  von dem abhängig ist, Fehlschläge hat), `dependency_roots` (Zählung für die Mail), `has_cycle`. `storage::pauses::sync`/`sync_all` (markiert pausierte
  Sensoren, schreibt `dependency_log` „active“/„ended“; Poller nach Zyklen mit Änderungen und alle 15 s), `set_dependency` (prüft Sensor und Zyklen,
  ein Master je Gerät), `dependency_entries` fürs Protokoll (ein Eintrag je Auslösung statt je Sensor). `alerting::deliver` verwirft Wechsel durch
  Abhängigkeit pausierter Sensoren, die Ausfallmail nennt die pausierten Objekte (`paused_dependents`). Seite `web::dependency` (`/dependency/<kind>/<id>`,
  nur Admins, `templates/dependency_form.html`), Knopf „Abhängigkeit“ auf Gruppen-, Geräte- und Sensorseite; Gerätevorlagen übernehmen den Master (`master`)
- Wartungsfenster wie die Zeitpläne in PRTG (2026-10-27, `docs/devices-and-groups.md`, „Maintenance windows“): `schedule` (reine Logik: `Window`
  `Weekly`/`Once`, `active_until`/`next_start` in der Zone `schedule::time_zone()`, gesetzt beim Start aus Config `time_zone`, sonst System; `Assignment`
  `Parent`/`Schedule`/`None`; Formularparser), `storage::schedules` (Migration `0020`: `schedules`, `schedule_windows`, `schedule_assignments` je
  Gruppe/Gerät/Sensor als eigene Tabelle, `schedule_log`; `load_into` füllt `Pauses` mit `ScheduleInfo.open_until`, `log_windows` aus `pauses::sync`),
  in `pause::Pauses` als `Source::Schedule` nach der Pause von Hand und vor den Abhängigkeiten (`group_by_hand`/`device_by_hand`, `maintenance`,
  `schedule_of`, `schedule_roots`); Seiten `web::schedules` (`/schedules`, `/schedules/<id>` mit Fenstern, `/maintenance/<kind>/<id>`, Knopf
  „Wartungsfenster“ im Menü „⋯ Aktionen“), Protokolleinträge in `web::log`
- Verfügbarkeit wie die Berichte in PRTG (2026-10-27, `docs/reports.md`): `availability` (reine Logik: `Totals` ms je Zustand, `percent` = (OK + Warnung) /
  (OK + Warnung + Ausfall), `split_hours`, `counted_span` (Lücke über `GAP_INTERVALS` = 3 Intervalle zählt nicht), `device` nach Master, sonst Ping, sonst
  Mittel), `storage::availability` (Migration `0021`: `availability_1h`/`_1d`; `count` aus `storage::status::record_poll` in derselben Transaktion, `rollup`
  und `apply_retention` aus `maintenance`, `totals` stundengenau mit Tagen als Rückfall), Seite `web::reports` (`/reports`, `/reports.csv`, alle Benutzer;
  Monate in `schedule::time_zone()`; `percent_text` rundet ab), Kacheln auf der Sensorseite (`reports::sensor_summary`)
- Änderungsprotokoll (2026-10-27, `docs/users-and-login.md`, „Audit log“): `audit` (`Action`, `Kind` mit `href`, `Changes`: `field` alt → neu nur bei
  Unterschied, `label`/`value_label` für Katalog-IDs, `limits` für Grenzwerte, `secret` nur „geändert“, `note`; `into_msg` → `Msg::List`), `storage::audit`
  (Migration `0022`: `audit_log`; `insert`, `list`, `apply_retention` aus `maintenance`, Config `retention_audit_days`), `web::audit` (`record` aus den
  Admin-Handlern **nach** der Änderung, übergeht „geändert“ ohne Unterschied; `write` für An-/Abmeldungen ohne `CurrentUser`; `Object::target` für
  Pausenziele; Seite `/audit` nur Admins, `?user=`). `CurrentUser.address` (Client-Adresse über `throttle::client_ip`). Fehlgeschlagene Anmeldungen nur
  mit dem Namen eines vorhandenen Kontos. Neue Admin-Handler müssen `audit::record` aufrufen; Secrets nie als Wert
- `devices` – Domäne und Validierung von Geräten und Gruppen (Host, Intervall, Gruppenbaum; `DeviceKind`: Gerätetyp, nur fürs Symbol in der Übersicht, Spalte `kind`, Migration `0015`); `crypto` – `SecretBox` (XChaCha20-Poly1305), Schlüsseldatei mit Rechteprüfung; `credentials` – Profil-Domäne und Validierung (`Secret` mit geschwärztem `Debug`)
- Geräte ohne SNMP: `Device.profile_id` ist `Option` (`None` = „Kein SNMP – nur Ping“, `has_snmp()`, `profile_label()`); interaktive SNMP-Aktionen holen die Zugangsdaten über `web::snmp_credentials` (Fehlermeldung für Ping-Geräte). Die Spalte wurde nachträglich nullbar: `storage::migrate` baut `devices` einmalig mit abgeschalteten Fremdschlüsseln neu auf (`allow_devices_without_profile`), weil das in einer sqlx-Migration (immer Transaktion) nicht geht
- `storage` – sqlx-Repository (`storage::users` für Benutzer und Sitzungen, `storage::credentials` für SNMP-Profile, `storage::groups`, `storage::devices`, `storage::sensors` für Sensoren, Kanäle und Messwerte, `storage::status` für Zustand und Ereignisse, `storage::pauses` für Pausen, Abhängigkeiten und ihr Protokoll, `storage::measurements` für Zeitreihen), Migrations, Rollups, Retention
- `poller` – Supervisor + eine Aufgabe je Gerät, die Gerät/Sensoren/Profil in jedem Zyklus neu lädt; Intervall je Sensor (`sensors.interval_secs`, Migration `0017`,
  `NULL` = Gerät; `SensorRecord::interval`, Formular `sensor_detail::set_interval`; GitHub-Issue #1): die Aufgabe merkt sich je Sensor die Fälligkeit (`SensorSet.due`),
  fragt je Zyklus nur fällige Sensoren ab und schläft bis zur nächsten Fälligkeit, höchstens ein Geräteintervall; verteilt Startzeiten über das Intervall
  (kein Burst zur vollen Minute); globale Nebenläufigkeit per `Semaphore` (`poll_concurrency`, `poller_enabled`); Timeout und Retries pro Gerät;
  ein langsames Gerät blockiert nie andere; nach geräteweitem SNMP-Fehler werden die übrigen SNMP-Sensoren des Zyklus übersprungen (`docs/poller.md`)
- `http_client` – `fetch` (eine Frist für alles, Weiterleitungen, Inhalt begrenzt), `parse_url` (nur http/https, keine Zugangsdaten in der URL), `connect_tcp`,
  `connect_tls` (prüft die Kette mit `webpki-roots`, meldet das Zertifikat und das Problem, `accept_expired`); Testserver `http_client::testserver`
- `update_check` – Hinweis auf neue Versionen (Config `update_check`, Code-Standard aus, ausgelieferte Config an): `spawn` fragt fünf Minuten nach dem Start, dann
  täglich `api.github.com/repos/<repository>/releases/latest` (`api_url` aus `CARGO_PKG_REPOSITORY`, Vorabversionen zählen nicht, Link muss auf die Releases
  des Repos zeigen), Ergebnis in `AppState::latest_release` (`Latest`), `CurrentUser.update` nur für Admins, Anzeige `.sidebar-update` in `templates/app.html`;
  Fehler nur auf `debug` (`docs/operations.md`, „Notice of new versions“)
- `sensors` – Trait `Sensor` (`poll` mit `PollContext`, liefert `Measurement` aus `ChannelReading`s und optional `note` als Meldung bei Erfolg; `value: None` = kein Sample) + Implementierung je Sensortyp: `sensors::snmp` (Uptime, Interface-Traffic mit 64-Bit-Fallback auf 32 Bit, Interface-Status, OID Gauge/Counter), `sensors::ping` (`PingClients`, geteilte ICMP-Sockets), `sensors::local` („Dieser Server“: `LocalMetric` CPU, Load, Speicher, Uptime, Dateisystem, Temperatur, Traffic aus `/proc`, `/sys` und `statvfs`; Typ `local`, Spalte `sensors.target`, Tabelle einmalig neu aufgebaut von `storage::migrate`/`allow_local_sensors`; Seite `web::server_values` unter `/devices/<id>/server`, erreichbar über den Typ „Dieser Server“ in „Sensor hinzufügen“ (jedes Gerät) und als Knopf bei Gerätetyp Server, Loopback-Adresse oder vorhandenen lokalen Sensoren; die Unit setzt kein `ProcSubset=pid`; `docs/sensors.md`), `sensors::service` (Dienste ohne SNMP, 2026-10-27: `http`, `tcp`, `dns`, `tls`; Einstellungen als JSON in `sensors.params`, `SensorSpec::params`/`from_params`;
  Fehlschläge als `SensorError::Check(Msg)`; Zertifikat mit Kanal `days`, `Unit::Days`, Grenzen 14/3 Tage über `SensorSpec::default_thresholds`; Formular mit
  `fieldset[data-kinds]` und `static/sensor-form.js`; `docs/sensors.md`, „Services“), `sensors::dns` (eigene A/AAAA-Abfrage über UDP an das Gerät),
  `sensors::spec` (`SensorSpec`: Validierung, Kanäle, `build()`, `parse_oid`). Neue Sensortypen brauchen einen Neuaufbau von `sensors`
  (`storage::current_sensors_table`, ausgelöst durch die neueste Spalte, kopiert die vorhandenen Spalten). `sensors::counter`: `CounterTracker` berechnet Raten aus 32-/64-Bit-Countern; Reset, Neustart (`sysUpTime` kleiner), unbestätigter 32-Bit-Wrap und unplausible Raten liefern `Rate::Skipped` statt eines Werts
- `maintenance` – Rollups (`samples_1h`, `samples_1d`) und Aufbewahrung (Config `retention_raw_days`, `retention_hourly_days`), vom Poller periodisch gestartet
- `status` – `Thresholds` (je Kanal warn/down über/unter, strikt, sofort ohne Entprellung), `ValueState`/`ChannelRules` (Bewertung benannter Statuswerte je Kanal, Tabelle `value_states`, Migration `0012`), `suggest_status` (Vorschlag aus dem Namen, beim Anlegen über `web::sensor_text::rate_new_sensor`), `assess_with` (Messergebnis + Grenzen + Bewertungen → `Assessment`; `assess` ohne Bewertungen), `advance` (Zustandsautomat: erster Fehler `Warning`, ab dem zweiten `Down`, Erfolg sofort `Up`), `device_status`; `storage::status::record_poll` schreibt Messwerte, Zustand und Ereignislog (`sensor_status`, `events`) in einer Transaktion
- `alerting` – `AlertEvent`, `is_alertworthy`, `compose` (in der Mail-Sprache `MailConfig.language`), `deliver`; `Notifier`-Aufgabe sammelt Wechsel 10 s lang zu einer Mail; vom Poller über eine `mpsc`-Queue gespeist (nie blockierend)
- `mail` – SMTP-Einstellungen, Validierung, Versand mit `lettre` (rustls, kein OpenSSL); `storage::mail` – Einstellungen (Passwort verschlüsselt, Kontext `smtp.password`) und Sendelog `notifications`; Seite `web::mail_settings`
- `web` – axum-Router, Handler, Templates (`templates/`), eingebettete Assets (`static/`), Login (`web::auth`,
  Extractor `CurrentUser`; POST-Handler prüfen `verify_csrf`, Admin-Handler `require_admin`), Startseite als Gruppenbaum
  (`web::overview`: Gruppen mit Sprungmarke `#group-<id>`; ein Baum wie in PRTG mit Linien und ⊟/⊞, Untergruppen als `Row::Start`/`End` verschachtelt mit Summen über den Teilbaum, Sensorplaketten je Zustand (`Counts::pills`), aufklappbar, Zustand in `localStorage` über `static/app.js`; Baum-Makros gemeinsam in `templates/tree.html`), Gruppenseite `/groups/<id>`
  (`overview::group`, `templates/group.html`: Teilbaum, Kacheln mit `/status/<filter>?group=<id>`, Aktionen; Ziel des Gruppennamens im Baum und der Pfadleiste
  `web::group_trail`/`device_trail.html` auf Geräte-, Sensor- und Gruppenseiten; Formulare kehren mit `back=group-<id>` zurück, `web::back_to_group`), Gruppen-, Geräte-, Profil- und Benutzerseiten (`web::groups`, `web::devices`, `web::credentials`, `web::users`: letzter Admin geschützt, eigenes Konto nicht löschbar, Passwortwechsel beendet die übrigen Sitzungen, Sprache je Benutzer; `web::account` „Mein Konto“ für alle: eigene Sprache, eigenes Passwort mit dem bisherigen, Fehlversuche über `login_throttle`), Sensorseiten mit Discovery und Mini-Graphen der letzten Stunde (`web::sensors`, `web::sparkline`: Linien bzw. Statusband als SVG), Detailseite mit JSON-Endpoint für uPlot, Tabelle „Messwerte“ wie die PRTG-Livedaten (`static/samples.js`, immer Rohwerte über `/sensors/<id>/samples`, `storage::measurements::recent_samples`) und Schwellwertformular (`web::sensor_detail`); Statuslisten hinter den Kacheln der Übersicht
  (`web::status_list`, `/status/all|up|warning|down|inactive`, sortierbar per Spaltenkopf `?sort=…&dir=…`, aktuelle Werte über `web::sensors::fresh_values`); oben rechts auf Übersicht, Statuslisten, Sensor- und Detailseite „Aktualisieren“ mit
  „Automatisch“ (alle 30 s wie PRTG, pausiert bei Eingaben; `templates/refresh.html`, `static/refresh.js`); Protokoll `/log` (`web::log`: sortierbar per Spaltenkopf wie die Statuslisten, Suchfeld über `static/table-filter.js`, `?device=<id>`/`?sensor=<id>` über den
  Knopf „Protokoll“ der Geräte- und Sensorseite; Warnungen, Ausfälle und
  ihr Ende, `storage::status::problem_log`, dazu Pausen aus `storage::pauses::log_entries` und Abhängigkeiten aus `storage::pauses::dependency_entries`; `events` hält nur die neuesten 500 Wechsel, `EVENTS_KEPT`)

### Messdaten

- Rohwerte in `samples` (channel_id, ts in ms UTC, value; der Sensor ergibt sich über `channels`); Rollups `samples_1h`, `samples_1d` (min/avg/max)
- Retention (konfigurierbar): Rohdaten 7 Tage, Stundenwerte 1 Jahr, Tageswerte unbegrenzt
- Counter → Rate: Differenz / Zeitdelta; Counter-Wrap und Reset (z. B. Gerätereboot, `sysUpTime` kleiner) erkennen
  und dann **keinen** Wert schreiben statt eines Ausreißers
- Zeitstempel intern UTC; Anzeige in der Zeitzone des Browsers (`<time class="ts">` aus `web::time_html`, umgeschrieben von `static/time.js`; Graphen in `static/chart.js`; E-Mails bleiben UTC)
- Access Points liefern nur 32-Bit-Counter (`ifTable`, kein `ifXTable`): Wrap bei 32 Bit sauber behandeln, Intervall passend wählen
- Interface-Indizes der Switches sind nicht fortlaufend (z. B. 49153): Namen über `ifName`/`ifDescr` auswerten

## Sicherheit

- SNMP ausschließlich lesend
- SNMP-Community/v3-Passwörter verschlüsselt in der DB (XChaCha20-Poly1305, Spaltenkontext als AAD); Schlüssel aus Datei mit Rechten 0600
  (`secret_key_file`, Server startet sonst nicht, Erzeugung mit `opennetmon key generate`); Entschlüsselung nur für den Poller;
  Profil-Seiten nur für Administratoren
- Secrets niemals loggen, nie in HTML ausgeben (Formulare zeigen nur „gesetzt/nicht gesetzt"), nie ins Änderungsprotokoll (nur „geändert“)
- Passwörter mit argon2; Session-Cookies `HttpOnly`, `SameSite=Strict`, `Secure` laut Config (`cookie_secure`, Standard `true`);
  CSRF-Schutz für alle POST-Formulare (Login: Double-Submit-Cookie, danach Token pro Sitzung)
- Sitzungen serverseitig; in der DB steht nur der SHA-256-Wert des Cookie-Tokens; kein Standardbenutzer, erster Admin per CLI
- Abmelden ohne Aktivität (Wunsch des Nutzers, 2026-10-24): Config `session_idle_minutes` (Standard 30, `0` = aus), zusätzlich zur festen Höchstdauer
  `session_ttl_hours`; `sessions.last_seen_at` (Migration `0018`, höchstens alle 15 s geschrieben), Prüfung im `CurrentUser`-Extractor (abgelaufen: Sitzung
  löschen, `/login?idle=1` mit Hinweis). Aktivität = Seitenaufruf (`Sec-Fetch-Mode: navigate`), POST und Klicks/Tasten über `static/session.js`
  (`POST /session`, höchstens einmal pro Minute; `GET /session` liefert die Restzeit und zählt nicht); **automatisches Aktualisieren zählt nicht**
  (Cookie `opennetmon_auto` von `static/refresh.js`), Skripte (Graph, Messwerte) auch nicht. Ausnahme je Benutzer `users.stay_signed_in`
  („angemeldet bleiben“, z. B. Wandbildschirm; nur Admins auf `/users/<id>`, Route `/users/<id>/idle`)
- Sicherheits-Header auf allen Antworten (CSP mit `base-uri 'self'` und `object-src 'none'`, `X-Frame-Options`, `nosniff`, `Referrer-Policy`,
  `Cache-Control: no-store` außer bei den statischen Dateien); Fehler von Seitenaufrufen (Accept `text/html`) zeigt `web::error_page` als Seite im Design (mit Seitenleiste bzw. Anmeldekarte; Meldung aus `error::ErrorMessage`, sonst je Status, nie axums englische Texte), JSON-Abfragen bleiben Klartext
- Begrenzung der Anmeldeversuche (`auth::throttle`): Fehlversuche je Benutzername (ab 5, bis 60 s Wartezeit) und je Adresse (ab 10, bis 15 min), Ablehnung mit 429 ohne Passwortprüfung, nur bremsen (nie dauerhaft sperren); hinter einem Proxy `client_ip_header` setzen (`docs/users-and-login.md`).
  Jeder Versuch zählt schon vor der Prüfung als Fehlversuch (`LoginThrottle::begin_attempt`, Prüfen und Zählen unter einer Sperre; Erfolg löscht),
  sonst kämen gleichzeitige Versuche alle durch. Argon2-Prüfungen nur über `AppState::check_password`: höchstens 4 gleichzeitig (`PASSWORD_CHECKS`, je ~19 MiB),
  Platz bleibt bis zum Ende der Prüfung belegt, nach 10 s ohne Platz 503 „login-busy“ (Sicherheitstest 2026-10-27: vorher 2,4 GB bei 200 Versuchen)
- Omada-SNMP ist nur authNoPriv/MD5 (keine Verschlüsselung, schwacher Hash): nur im privaten Netz, Passwort lang und einzigartig wählen
- Ausgelieferte Config (Paket, `install.sh`): `listen = "0.0.0.0:8443"` mit eingebautem HTTPS (seit 0.3.0, Wunsch des Nutzers: direkter Zugriff per IP und Port, zwingend HTTPS);
  Zertifikat `/etc/opennetmon/tls-cert.pem`/`tls-key.pem` bei der Installation selbst signiert erzeugt (`hostname -f`, `hostname -I`), ersetzbar durch ein eigenes
  (`docs/operations.md`, „HTTPS“: eigene CA mit openssl, Let's Encrypt, Port 443 per Drop-in mit `CAP_NET_BIND_SERVICE`). Ohne `tls_*` Standard-Bind `127.0.0.1:8080`
  für einen Reverse Proxy (Caddy oder nginx); kein HSTS (gälte für alle Ports des Hosts)
- Eingaben (IP, OID, Intervalle) serverseitig validieren

## Betrieb auf Ubuntu Server

Vollständige Anleitung: `docs/operations.md`. Dateien unter `deploy/`:

- Release-Build als statisches Binary: Target `x86_64-unknown-linux-musl` (vermeidet glibc-Versionskonflikte zwischen Zorin und dem Server)
- Debian-Paket (empfohlen für Ubuntu/Debian amd64): `dev/deb.sh` baut mit `cargo-deb` (`[package.metadata.deb]` in `Cargo.toml`) `target/debian/opennetmon_<version>-1_amd64.deb`;
  wie `install.sh`, aber unter `/usr` (`/usr/bin`, `/usr/sbin/opennetmon-backup`, Units in `/usr/lib/systemd/system`, sysctl in `/usr/lib/sysctl.d`); Units und Sicherungsskript
  erzeugt `dev/deb.sh` per `sed` aus `deploy/` nach `target/deb-assets/` (`deploy/` bleibt die einzige Quelle); Maintainer-Skripte `deploy/deb/postinst`/`postrm` (Benutzer,
  `dpkg-statoverride` für `/etc/opennetmon` und `config.toml`, Schlüssel, HTTPS-Zertifikat, `/var/lib/opennetmon`, `/var/backups/opennetmon`, Hinweis auf Reste von `install.sh`; `purge` löscht Konfiguration, Schlüssel und Zertifikat,
  nie Datenbank, Sicherungen oder Benutzer); `opennetmon.service` aktiviert, bei der Erstinstallation nicht gestartet, bei Updates `try-restart`
- `deploy/install.sh` (wiederholbar; `DESTDIR=… ` für einen Trockenlauf): Systembenutzer `opennetmon`, `/usr/local/bin/opennetmon` (alte Version als `.previous`),
  `/etc/opennetmon/config.toml` und `secret.key` (werden nie überschrieben), Unit, sysctl für Ping, täglicher Backup-Timer
- `deploy/opennetmon.service`: eigener Benutzer, `StateDirectory=opennetmon` (`/var/lib/opennetmon`), strenges Hardening (`ProtectSystem=strict`, `NoNewPrivileges`, leere Capabilities,
  Syscall-Filter `@system-service` ohne `@privileged @resources`, `MemoryDenyWriteExecute`); `systemd-analyze security` ≈ 1,3; als transienter Dienst real getestet
- Pfade: Config `/etc/opennetmon/config.toml`, Daten `/var/lib/opennetmon/`, Sicherungen `/var/backups/opennetmon`, Logs über journald
- Ping über `net.ipv4.ping_group_range` (`deploy/99-opennetmon-ping.conf`); Alternative `CAP_NET_RAW` (Hinweis in der Unit); der Poller warnt beim Start, wenn keine ICMP-Sockets möglich sind
- `opennetmon backup <datei>` (SQLite `VACUUM INTO`, Modus 0600, überschreibt nie, ohne Schlüssel) und `deploy/opennetmon-backup.sh` + `.timer` (täglich, 14 Kopien)
- Reverse-Proxy-Beispiele: `deploy/Caddyfile.example`, `deploy/nginx.conf.example` (mit Login-Begrenzung)
- Health-Endpoint `/healthz` (ohne Auth, ohne interne Details; `curl -sk https://127.0.0.1:8443/healthz`)
- Paket getestet im Podman-Container (Image `opennetmon-debtest`, Ubuntu 24.04 mit systemd; `/usr/sbin/policy-rc.d` darin löschen): Installation, Update, remove, purge, Umstieg

## Konventionen

- Code, Bezeichner, Commit-Messages und Code-Kommentare: Englisch
- UI-Texte: über die Fluent-Kataloge in `locales/` (Deutsch ist Ausgangssprache, Englisch muss mitgepflegt werden; keine festen Texte mehr in Templates
  oder Rust-Code – Seiten nutzen `t.get("id")`, Fachlogik liefert `Msg`, JS nutzt `T('key')`). `Msg::Text` nur für fremde Texte (Gerät, SMTP-Server);
  Benachrichtigungs-Mails in der Sprache von `mail_settings.language` (eine für alle Empfänger). Dokumentation (`docs/`, `mibs/README.md`) und `README.md`: **Englisch** (seit 2026-10-22, vorher war `docs/` deutsch); nur CLAUDE.md und PLAN.md bleiben deutsch
- Kein `unwrap()`/`expect()` außerhalb von Tests und Startcode; Fehler über eigenen Error-Typ mit `IntoResponse`
- Kleine, thematisch abgeschlossene Commits; nach jedem MVP-Schritt lauffähiger Stand
- Neue Sensortypen nur über den `Sensor`-Trait, keine Sonderfälle im Poller
- AGPL Abschnitt 13: Seitenleiste (`templates/app.html`) und Anmeldeseite verlinken den Quellcode (`web::SOURCE_URL` = `repository` aus `Cargo.toml`)
  mit Version (`web::VERSION`); diese Links nicht entfernen. Der Installer legt `LICENSE` und `THIRD-PARTY-LICENSES.txt` nach `/usr/local/share/doc/opennetmon/`,
  das Paket `copyright` (aus `LICENSE`) und `THIRD-PARTY-LICENSES.txt` nach `/usr/share/doc/opennetmon/`

## Tests

- CI: `.github/workflows/ci.yml` (fmt, clippy, Tests, `third-party-licenses.py --check`; Job `audit` mit `cargo audit` bei Push/PR und montags per `schedule`,
  `cargo-audit` per `cargo install` in fester Version mit Cache, weil nur `actions/*` erlaubt; `dev/release.sh` prüft es auch; bei Pull Requests zusätzlich DCO-Prüfung `Signed-off-by`
  des Autors); Actions nur von `actions/*`, auf Commit-Hash gepinnt. Beitragsregeln in `CONTRIBUTING.md` (Englisch), Meldeweg für Lücken in `SECURITY.md`
- Unit-Tests für Counter-/Ratenberechnung (inkl. Wrap/Reset), Statuslogik, Config-Parsing
- Integrationstests gegen lokalen `snmpd` (net-snmp) als Testagent (`tests/snmp_local.rs`, überspringen sich ohne Agent oder
  ohne `OPENNETMON_TEST_V3_*`); Gerätetests gegen echte Omada-Hardware nur manuell (`tests/omada_manual.rs`, alle `#[ignore]`; u. a. `sensors_work_on_switch_and_access_point` und `poller_cycle_against_real_devices`, `interface_discovery_on_switch_and_access_point`)
- Handler-Tests mit `axum`/`tower::ServiceExt::oneshot`, gemeinsame Helfer in `src/web/testutil.rs`; SNMP-Tests ohne Netz über den
  Fake-v2c-Agenten `src/snmp/testagent.rs` (setzbare OID-Tabelle, GET/GETNEXT/GETBULK)

## Befehle

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo run -- --config ./dev/config.toml
cargo run -- --config ./dev/config.toml key generate                    # einmalig, vor dem ersten Serverstart
cargo run -- --config ./dev/config.toml user add <name> --role admin    # auch: user passwd, user list
cargo run -- --config ./dev/config.toml backup ./dev/data/backup.db      # konsistente Kopie der Datenbank
cargo run -- --config ./dev/config.toml tls generate --cert ./dev/data/tls-cert.pem --key ./dev/data/tls-key.pem   # Zertifikat zum Testen von HTTPS
cargo run --example snmp_probe -- -t <ip>:161 v3 -u <user> --level auth-no-priv --auth md5   # Passwörter nur per Umgebung
cargo test --test omada_manual -- --ignored --nocapture                 # manuell, braucht omada.env (siehe Datei)
dev/mib-oracle.sh dev/data/mibs/tplink/*.mib                          # MIB-Auflösung gegen net-snmp (snmptranslate) prüfen
python3 -I dev/third-party-licenses.py                                 # THIRD-PARTY-LICENSES.txt neu erzeugen (--check: nur prüfen)
cargo audit                                                            # bekannte Lücken in Abhängigkeiten (RustSec; Ausnahmen mit Begründung in .cargo/audit.toml)
dev/deb.sh                                                             # Debian-Paket bauen (target/debian/), braucht cargo install cargo-deb --locked
dev/release.sh                                                         # Release bauen und prüfen (Archiv, .deb und SHA256SUMS in target/dist/)
dev/release.sh --publish --notes <datei>                               # zusätzlich Tag vX.Y.Z pushen und GitHub-Release anlegen (nur auf Ansage)
cargo build --release --target x86_64-unknown-linux-musl
```

Releases: Version in `Cargo.toml` erhöhen und committen, dann `dev/release.sh --publish --notes <datei>` (Release-Notizen auf **Englisch** seit 2026-10-22; v0.1.0–v0.4.1 sind zweisprachig, Englisch oben; prüft sauberen,
gepushten Stand, fmt/clippy/Tests, Fremdlizenzen, Installer-Probelauf aus dem Archiv, Inhalt des .deb; seit 0.4.4 reguläre Releases, Wunsch des Nutzers – vorher alle als Vorabversion; `--prerelease` markiert bei Bedarf eine als Vorabversion). Bisher: **v0.1.0** (2026-10-19), **v0.2.0** (2026-10-20, erstmals mit .deb), **v0.3.0** (2026-10-20, eingebautes HTTPS), **v0.3.1** (2026-10-20, Statuslisten hinter den Kacheln; Repo privat), **v0.4.0** (2026-10-21, Pausieren, Protokoll, Gerätebaum mit Gerätetypen), **v0.4.1** (2026-10-21, Sensoren „Dieser Server“), **v0.4.2** (2026-10-22, Gruppenseiten, Doku auf Englisch), **v0.4.3** (2026-10-23, Intervall je Sensor, Messwerttabelle mit Rohwerten, Fehlerseiten im Design), **v0.4.4** (2026-10-24, erstes reguläres Release: Abmelden ohne Aktivität, MIB-Dateien nach Hersteller, Suche und Sortierung im Protokoll), **v0.5.0** (2026-10-27, Abhängigkeiten wie in PRTG, Handy-Ansicht, Symbole in den Listen, Behebungen aus dem Sicherheitstest).

`README.md` (GitHub-Startseite) ist seit 2026-10-19 **nur noch Englisch** und wird nur auf Englisch gepflegt; bei Meilensteinen mitpflegen: Status-Tabelle, Konfigurationsbeispiel und Dokumentationsliste müssen zum Code passen.

**Vor jedem Abschluss einer Aufgabe:** `cargo fmt`, `cargo clippy --all-targets -- -D warnings`
und `cargo test` müssen fehlerfrei durchlaufen.

## Offene Punkte (vom Nutzer zu klären)

- Angaben zum Heimnetz des Nutzers (Site, Adressen, SNMP-Benutzer und offene Punkte dazu) stehen nur lokal in `CLAUDE.local.md` (gitignored)
  und `~/.config/opennetmon-dev/omada.env`, nie im Repo oder auf Drive; Tests nutzen neutrale Werte (`192.0.2.x`, Benutzer `monitor`)
- ER605: läuft jetzt als Gerät ohne SNMP (nur Ping); später zusätzlich über die Omada-API?
- Schlüsseldatei (`secret.key`): Sicherung getrennt von der Datenbank einplanen; Schlüsselrotation ist noch nicht umgesetzt
- Update-Möglichkeit: heute neues `.deb` per `apt install`; Hinweis auf neue Versionen in der Seitenleiste für Admins umgesetzt (2026-10-27, I), später evtl. apt-Repository
- In v0.5.0: mobile Ansicht und Symbole statt Text in den Listen (2026-10-26), Sensorlisten nur mit dem Namen (Beschreibung nur auf der
  Detailseite), Abhängigkeiten wie in PRTG (2026-10-27, `PLAN.md`, „Nächste Vorhaben“, B) und die Behebungen aus dem Sicherheitstest vom 2026-10-27
  (`PLAN.md`, „Verifikation“). Nach v0.5.0 umgesetzt (noch nicht released): Knopfleisten der Geräte-, Gruppen- und Sensorseite mit
  Pausieren/Fortsetzen und Protokoll sichtbar, die Einrichtung im Menü „⋯ Aktionen“ auf jeder Breite (2026-10-27, `PLAN.md`, „Nächste Vorhaben“, D)
- Umgesetzt nach v0.5.0 (noch nicht released, 2026-10-27): E Dienst-Sensoren, H `cargo audit` in der CI, I Hinweis auf neue Versionen,
  J Wartungsfenster, G Verfügbarkeit und Berichte (ohne Monatsmail), M Änderungsprotokoll
- Geplant (2026-10-27, `PLAN.md`, „Weitere Vorhaben E–M“; E, G, H, I, J, M jetzt, F, K, L in einer späteren Version): E Sensoren HTTP(S)/TCP/DNS/TLS-Zertifikat,
  F Push (ntfy, Gotify, Telegram, Webhook), G Verfügbarkeit und Berichte, H `cargo audit` in der CI, I Hinweis auf neue Versionen, J Wartungsfenster nach Zeitplan,
  K Zwei-Faktor-Anmeldung (TOTP), L Rotation von `secret.key`, M Änderungsprotokoll

Geklärt: GitHub-Repo `tomCSLK/opennetmon` ist seit 2026-10-22 **öffentlich** (Historie ohne Secrets; Site-Name, SNMP-Benutzer und private Adressen
stehen nur noch in alten Commits, wie vereinbart nicht umgeschrieben; Autor-Adresse bleibt), mit „Private vulnerability reporting“, Dependabot-Warnungen,
Secret Scanning und Push Protection; Beiträge nach **DCO** (`Signed-off-by`, CI prüft es bei Pull Requests; 2026-10-21) – eine spätere Doppellizenzierung
bräuchte damit die Zustimmung der Beitragenden; Server hat Internetzugang und betreibt weitere Programme, die es brauchen (2026-10-21); Name opennetmon (mit Logo), Lizenz AGPL-3.0-or-later, Zielumgebung Omada (7 Geräte, im MVP etwa 70 Sensoren, SQLite reicht), SNMPv3 authNoPriv/MD5, Sprache Rust.
