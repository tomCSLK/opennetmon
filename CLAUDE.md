# CLAUDE.md – PRTG Clone für Linux (Arbeitstitel)

> Hinweis: „PRTG" ist eine Marke von Paessler. Der Titel ist nur ein interner Arbeitstitel.
> Crate-, Binary- und Paketname: `netmon` (Platzhalter, später umbenennbar).

## Ziel

Agentloses Netzwerk-Monitoring per SNMP mit Web-Oberfläche, inspiriert vom Bedienkonzept von PRTG.
Läuft headless auf einem **Ubuntu Server ohne GUI**; Bedienung ausschließlich über den Browser.
Entwickelt wird auf Zorin OS (Desktop), betrieben auf Ubuntu Server.

Zielumgebung: privates Netz mit TP-Link Omada (Software-Controller, Site `MCS6`): Gateway ER605, drei Smart-Switches
(2× SG2008, 1× SG2008P), drei Access Points (1× EAP225, 2× EAP653). SNMP wird im Controller site-weit konfiguriert
und ist dort nur als **SNMPv3 authNoPriv mit MD5** aktiv (v1/v2c aus, kein Privacy-Passwort). Gemessen im Spike:
Switches und Access Points antworten, der ER605 nicht. Switches liefern 64-Bit-Counter (`ifXTable`), Access Points nur
32-Bit-Counter (`ifTable`). Die Omada-Controller-API ist **nicht** Teil des SNMP-MVP.

## Sprachentscheidung: Rust (entschieden)

**Phase 0 (Spike) ist abgeschlossen, Entscheidung am 2026-10-15: Rust bleibt.** `examples/snmp_probe` fragt
`sysDescr`, `sysUpTime`, `ifHCInOctets` und GETBULK-Walks über `ifTable`/`ifXTable` per SNMPv2c und SNMPv3
(authNoPriv und authPriv) ab und lief gegen den lokalen `snmpd` (SHA-256/AES-128) sowie gegen alle SNMP-fähigen Omada-Geräte
(authNoPriv/MD5). Details und Messwerte: `docs/decisions/0001-sprache.md`.

Der Python-Fallback (unten) ist **nicht** gewählt und nur noch als Dokumentation der Alternative erhalten.
Die Sprache nicht eigenmächtig wechseln – bei grundlegenden Problemen Befund dokumentieren und nachfragen.

## Stack (Rust)

- Runtime/Web: `tokio`, `axum`, `tower-http` (Static, Compression, Trace)
- Templates/UI: `askama` (server-seitiges HTML), **htmx** für Interaktivität, **uPlot** für Graphen.
  JS/CSS lokal vendoren (kein CDN – Server steht ggf. ohne Internet), per `rust-embed` ins Binary.
  Kein npm, kein WASM-Frontend.
- Design: `static/app.css` ist ein kleines Designsystem (Farb-Tokens für hell/dunkel, Seitenleiste, Kacheln, Karten, Badges, Tabellen, Formulare);
  Schema folgt dem System, Umschalter in der Seitenleiste (`static/theme.js` im `<head>` gegen Aufblitzen, `static/app.js` für Umschalter, aktiven Menüpunkt
  und Handy-Menü). Keine Inline-Styles (CSP): Breiten z. B. der Sensor-Statusleiste als SVG-Attribute. Graphen lesen ihre Farben aus den CSS-Variablen
  und zeichnen bei Schemawechsel neu (Event `netmon:theme`)
- Statische Dateien (`web::assets`): `Cache-Control: no-cache` + ETag (SHA-256 des eingebetteten Inhalts), Antwort `304` bei unverändertem Stand –
  neue Versionen gelten nach einem Update sofort, ohne Versionsnummern in den Adressen pflegen zu müssen
- SNMP: `snmp2` mit Features `tokio`, `crypto-rust` (keine OpenSSL-Abhängigkeit) und **`heap_buffers`** (Pflicht: sonst liegt ein 64-KB-Puffer
  auf dem Stack und überläuft in Tokio-Threads mit 2 MB Stack)
- Ping: `surge-ping` (0.9, ein Socket je Adressfamilie für alle Ziele); bevorzugt unprivilegierte ICMP-Sockets (`net.ipv4.ping_group_range`), sonst `CAP_NET_RAW`
- DB: SQLite über `sqlx` (Migrations in `migrations/`, WAL-Modus)
- Uploads: axum-Feature `multipart` (nutzt `multer`) für MIB-Dateien
- `serde_json` für die Regeln der Gerätevorlagen (war über axum schon im Build)
- Mail: `lettre` 0.11 (Features `builder`, `smtp-transport`, `tokio1`, `tokio1-rustls-tls`; ohne native-tls/OpenSSL, damit der musl-Build statisch bleibt)
- Sonstiges: `serde`, `toml` (Config), `clap` (CLI), `tracing` + `tracing-subscriber`,
  `thiserror` (Lib-Fehler) / `anyhow` (nur in `main`), `argon2` (Passwörter, API ab 0.6: `hash_password`
  erzeugt den Salt selbst), `sha2` (Session-Token-Hash), `subtle` (Vergleich in konstanter Zeit), `rand` (Tokens),
  `rpassword` (Passwortabfrage in der CLI), `chacha20poly1305` (XChaCha20-Poly1305 für gespeicherte SNMP-Secrets; API ab 0.11:
  `XNonce::generate()`, `Key::from([u8; 32])`, AAD über `Payload`)
- Rust Edition 2024, stabile Toolchain
- `rust-embed` braucht das Feature `mime-guess`; `sqlx` das Feature `macros` für `migrate!` (keine Compile-Time-Queries)
- `snmp2` hat keinen eingebauten Timeout: jede Anfrage selbst mit `tokio::time::timeout` begrenzen

Neue Abhängigkeiten nur mit kurzer Begründung im Commit; keine unmaintained Crates.

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
   (`docs/snmp-profile.md`, `docs/geraete-und-gruppen.md`), inklusive Verbindungstest je Gerät
2. Sensortypen: Ping, SNMP Uptime, Interface-Traffic (IF-MIB, 64-Bit-HC-Counter), Interface-Status
   (`ifOperStatus`), benutzerdefinierter OID-Sensor (Gauge/Counter)
3. Auto-Discovery der Interfaces per Walk; Nutzer wählt, welche als Sensoren angelegt werden – **umgesetzt** (`docs/sensoren.md`);
   darüber hinaus MIB-Import und „Werte suchen“ (PoE, CPU, Speicher, Hardware-Sensoren; neue Sensoren inaktiv) – **umgesetzt** (`docs/mibs.md`)
4. Schwellwerte je Kanal → Statusberechnung; Statusübersicht und Detailseite mit Graph (1h/24h/7d/30d) – **umgesetzt** (`docs/graphen-und-schwellwerte.md`)
5. Login (lokale Benutzer), Rolle Admin/Leser – **umgesetzt** (Anlegen per CLI, siehe `docs/benutzer-und-login.md`)
6. Benachrichtigung per E-Mail (SMTP) bei Statuswechsel – **umgesetzt**, Einstellungen in der Oberfläche (`docs/benachrichtigung.md`)

**Nicht im MVP:** SNMP-Traps, NetFlow/sFlow, verteilte Probes, Netzwerkkarten/Maps, SNMP SET,
Mandantenfähigkeit, Omada-Controller-API, Herstellermibs außer gezielt benötigten TP-Link-OIDs.

## Architektur

Ein Binary, ein Prozess, Module:
- `config` – TOML-Datei + ENV-Overrides (`NETMON_LISTEN`, `NETMON_DATABASE_PATH`, `NETMON_LOG_FILTER`, `NETMON_SECRET_KEY_FILE`); `poller_enabled`, `poll_concurrency`
- `auth` – Rollen, Argon2-Hashing, Session-/CSRF-Tokens, Validierung von Benutzername und Passwort
- `snmp` – `SnmpClient` (GET und GETBULK-`walk` mit Timeout/Retries, v2c und v3, `heap_buffers`), `probe` für den Verbindungstest; `discovery` – Interface-Discovery über IF-MIB-Walks (`merge` ist reine Logik); Poller und Sensoren bauen darauf auf
- `mib` – MIB-Parser (`mib::lexer`, `mib::parser`, tolerant) und Index (`MibIndex`: Auflösung über Module, Textual Conventions, Tabellen inkl. AUGMENTS,
  Suche, Name ↔ OID, `oid_target` für das Sensorformular); mitgelieferte Standard-MIBs in `mibs/` (per `rust-embed`), Uploads in `mib_files`;
  Index im `AppState` (`mib_index()`, `reload_mibs()`), Seite `web::mibs`, Anzeige-Helfer `web::sensor_text` (Namen, Klartext, Einheiten) (`docs/mibs.md`); `mib::catalog` – bekannte Tabellen (PoE, CPU, Speicher, HOST-RESOURCES, ENTITY-SENSOR), Einheiten aus Beschreibungen
- `ordering` – Anzeigereihenfolge: `natural_cmp` (Zahlen der Größe nach), `position_cmp` (Handordnung zuerst), `reorder` aus Formularpositionen;
  Spalte `position` in `groups`/`devices`/`sensors` (`NULL` = automatisch, beim Verschieben in eine andere Gruppe zurückgesetzt), Schreiben über
  `storage::write_positions`; Seiten `web::order` (`/groups/<id>/order`, 0 = oberste Ebene; `/devices/<id>/sensors/order`) mit `static/order.js`;
  Sensoren automatisch nach Typ, dann Name; Umbenennen von Sensoren auf der Detailseite (`sensor_detail::rename`)
- `templates` – Gerätevorlagen: `Body` (Regeln als JSON in `device_templates`, Migration `0011`), `build` aus den Sensoren eines Geräts,
  `plan` (reine Logik: nur Fehlendes, Übersprungenes mit Grund), `apply` (Interfaces lesen, OIDs per GET prüfen, Sensoren mit Schwellwerten anlegen),
  `detect` über `sysObjectID`; `storage::templates`, `devices.template_id`; Seiten `web::templates` (`/templates`, `/devices/<id>/template/new|apply`),
  Auswahl „automatisch erkennen“ beim Anlegen eines Geräts (`docs/geraetevorlagen.md`)
- `value_discovery` – Tabellen/Gruppen walken und Sensorvorschläge bilden (Namen, Faktor, Einheit); Seite `web::sensor_values`, neue Sensoren standardmäßig **inaktiv** (`enabled = false`, Status „Inaktiv“), Sammelaktionen `web::sensors::bulk`
- `devices` – Domäne und Validierung von Geräten und Gruppen (Host, Intervall, Gruppenbaum); `crypto` – `SecretBox` (XChaCha20-Poly1305), Schlüsseldatei mit Rechteprüfung; `credentials` – Profil-Domäne und Validierung (`Secret` mit geschwärztem `Debug`)
- `storage` – sqlx-Repository (`storage::users` für Benutzer und Sitzungen, `storage::credentials` für SNMP-Profile, `storage::groups`, `storage::devices`, `storage::sensors` für Sensoren, Kanäle und Messwerte, `storage::status` für Zustand und Ereignisse, `storage::measurements` für Zeitreihen), Migrations, Rollups, Retention
- `poller` – Supervisor + eine Aufgabe je Gerät, die Gerät/Sensoren/Profil in jedem Zyklus neu lädt; verteilt Startzeiten über das Intervall
  (kein Burst zur vollen Minute); globale Nebenläufigkeit per `Semaphore` (`poll_concurrency`, `poller_enabled`); Timeout und Retries pro Gerät;
  ein langsames Gerät blockiert nie andere; nach geräteweitem SNMP-Fehler werden die übrigen SNMP-Sensoren des Zyklus übersprungen (`docs/poller.md`)
- `sensors` – Trait `Sensor` (`poll` mit `PollContext`, liefert `Measurement` aus `ChannelReading`s; `value: None` = kein Sample) + Implementierung je Sensortyp: `sensors::snmp` (Uptime, Interface-Traffic mit 64-Bit-Fallback auf 32 Bit, Interface-Status, OID Gauge/Counter), `sensors::ping` (`PingClients`, geteilte ICMP-Sockets), `sensors::spec` (`SensorSpec`: Validierung, Kanäle, `build()`, `parse_oid`). `sensors::counter`: `CounterTracker` berechnet Raten aus 32-/64-Bit-Countern; Reset, Neustart (`sysUpTime` kleiner), unbestätigter 32-Bit-Wrap und unplausible Raten liefern `Rate::Skipped` statt eines Werts
- `maintenance` – Rollups (`samples_1h`, `samples_1d`) und Aufbewahrung (Config `retention_raw_days`, `retention_hourly_days`), vom Poller periodisch gestartet
- `status` – `Thresholds` (je Kanal warn/down über/unter, strikt, sofort ohne Entprellung), `assess` (Messergebnis + Grenzen → `Assessment`), `advance` (Zustandsautomat: erster Fehler `Warning`, ab dem zweiten `Down`, Erfolg sofort `Up`), `device_status`; `storage::status::record_poll` schreibt Messwerte, Zustand und Ereignislog (`sensor_status`, `events`) in einer Transaktion
- `alerting` – `AlertEvent`, `is_alertworthy`, `compose`, `deliver`; `Notifier`-Aufgabe sammelt Wechsel 10 s lang zu einer Mail; vom Poller über eine `mpsc`-Queue gespeist (nie blockierend)
- `mail` – SMTP-Einstellungen, Validierung, Versand mit `lettre` (rustls, kein OpenSSL); `storage::mail` – Einstellungen (Passwort verschlüsselt, Kontext `smtp.password`) und Sendelog `notifications`; Seite `web::mail_settings`
- `web` – axum-Router, Handler, Templates (`templates/`), eingebettete Assets (`static/`), Login (`web::auth`,
  Extractor `CurrentUser`; POST-Handler prüfen `verify_csrf`, Admin-Handler `require_admin`), Startseite als Gruppenbaum
  (`web::overview`), Gruppen-, Geräte- und Profilseiten (`web::groups`, `web::devices`, `web::credentials`), Sensorseiten mit Discovery (`web::sensors`), Detailseite mit JSON-Endpoint für uPlot, Tabelle „Messwerte“ wie die PRTG-Livedaten (`static/samples.js`) und Schwellwertformular (`web::sensor_detail`)

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
  (`secret_key_file`, Server startet sonst nicht, Erzeugung mit `netmon key generate`); Entschlüsselung nur für den Poller;
  Profil-Seiten nur für Administratoren
- Secrets niemals loggen, nie in HTML ausgeben (Formulare zeigen nur „gesetzt/nicht gesetzt")
- Passwörter mit argon2; Session-Cookies `HttpOnly`, `SameSite=Strict`, `Secure` laut Config (`cookie_secure`, Standard `true`);
  CSRF-Schutz für alle POST-Formulare (Login: Double-Submit-Cookie, danach Token pro Sitzung)
- Sitzungen serverseitig; in der DB steht nur der SHA-256-Wert des Cookie-Tokens; kein Standardbenutzer, erster Admin per CLI
- Sicherheits-Header auf allen Antworten (CSP, `X-Frame-Options`, `nosniff`, `Referrer-Policy`)
- Begrenzung der Anmeldeversuche (`auth::throttle`): Fehlversuche je Benutzername (ab 5, bis 60 s Wartezeit) und je Adresse (ab 10, bis 15 min), Ablehnung mit 429 ohne Passwortprüfung, nur bremsen (nie dauerhaft sperren); hinter einem Proxy `client_ip_header` setzen (`docs/benutzer-und-login.md`)
- Omada-SNMP ist nur authNoPriv/MD5 (keine Verschlüsselung, schwacher Hash): nur im privaten Netz, Passwort lang und einzigartig wählen
- Standard-Bind `127.0.0.1:8080`; TLS über vorgeschalteten Reverse Proxy (Caddy oder nginx)
- Eingaben (IP, OID, Intervalle) serverseitig validieren

## Betrieb auf Ubuntu Server

Vollständige Anleitung: `docs/betrieb.md`. Dateien unter `deploy/`:

- Release-Build als statisches Binary: Target `x86_64-unknown-linux-musl` (vermeidet glibc-Versionskonflikte zwischen Zorin und dem Server)
- `deploy/install.sh` (wiederholbar; `DESTDIR=… ` für einen Trockenlauf): Systembenutzer `netmon`, `/usr/local/bin/netmon` (alte Version als `.previous`),
  `/etc/netmon/config.toml` und `secret.key` (werden nie überschrieben), Unit, sysctl für Ping, täglicher Backup-Timer
- `deploy/netmon.service`: eigener Benutzer, `StateDirectory=netmon` (`/var/lib/netmon`), strenges Hardening (`ProtectSystem=strict`, `NoNewPrivileges`, leere Capabilities,
  Syscall-Filter `@system-service` ohne `@privileged @resources`, `MemoryDenyWriteExecute`); `systemd-analyze security` ≈ 1,3; als transienter Dienst real getestet
- Pfade: Config `/etc/netmon/config.toml`, Daten `/var/lib/netmon/`, Sicherungen `/var/backups/netmon`, Logs über journald
- Ping über `net.ipv4.ping_group_range` (`deploy/99-netmon-ping.conf`); Alternative `CAP_NET_RAW` (Hinweis in der Unit); der Poller warnt beim Start, wenn keine ICMP-Sockets möglich sind
- `netmon backup <datei>` (SQLite `VACUUM INTO`, Modus 0600, überschreibt nie, ohne Schlüssel) und `deploy/netmon-backup.sh` + `.timer` (täglich, 14 Kopien)
- Reverse-Proxy-Beispiele: `deploy/Caddyfile.example`, `deploy/nginx.conf.example` (mit Login-Begrenzung)
- Health-Endpoint `/healthz` (ohne Auth, ohne interne Details)

## Konventionen

- Code, Bezeichner, Commit-Messages und Code-Kommentare: Englisch
- UI-Texte und Projektdokumentation (`docs/`): Deutsch
- Kein `unwrap()`/`expect()` außerhalb von Tests und Startcode; Fehler über eigenen Error-Typ mit `IntoResponse`
- Kleine, thematisch abgeschlossene Commits; nach jedem MVP-Schritt lauffähiger Stand
- Neue Sensortypen nur über den `Sensor`-Trait, keine Sonderfälle im Poller

## Tests

- Unit-Tests für Counter-/Ratenberechnung (inkl. Wrap/Reset), Statuslogik, Config-Parsing
- Integrationstests gegen lokalen `snmpd` (net-snmp) als Testagent (`tests/snmp_local.rs`, überspringen sich ohne Agent oder
  ohne `NETMON_TEST_V3_*`); Gerätetests gegen echte Omada-Hardware nur manuell (`tests/omada_manual.rs`, alle `#[ignore]`; u. a. `sensors_work_on_switch_and_access_point` und `poller_cycle_against_real_devices`, `interface_discovery_on_switch_and_access_point`)
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
cargo run --example snmp_probe -- -t <ip>:161 v3 -u <user> --level auth-no-priv --auth md5   # Passwörter nur per Umgebung
cargo test --test omada_manual -- --ignored --nocapture                 # manuell, braucht omada.env (siehe Datei)
dev/mib-oracle.sh dev/data/mibs/tplink/*.mib                          # MIB-Auflösung gegen net-snmp (snmptranslate) prüfen
cargo build --release --target x86_64-unknown-linux-musl
```

`README.md` (GitHub-Startseite, Deutsch) bei Meilensteinen mitpflegen: Status-Tabelle, Konfigurationsbeispiel und Dokumentationsliste müssen zum Code passen.

**Vor jedem Abschluss einer Aufgabe:** `cargo fmt`, `cargo clippy --all-targets -- -D warnings`
und `cargo test` müssen fehlerfrei durchlaufen.

## Offene Punkte (vom Nutzer zu klären)

- Endgültiger Projektname
- Ist der Server vom Internet getrennt? (relevant für Updates und E-Mail-Versand)
- SNMPv3-Passwort des Omada-Benutzers ist kurz (8 Zeichen) und stand einmal im Chat: im Controller auf ein längeres
  Passwort ändern und lokal in `~/.config/netmon-dev/omada.env` nachtragen (nie ins Repo oder auf Drive)
- ER605 (kein SNMP): nur per Ping überwachen oder später über die Omada-API?
- Schlüsseldatei (`secret.key`): Sicherung getrennt von der Datenbank einplanen; Schlüsselrotation ist noch nicht umgesetzt
- uPlot 1.6.32 ist eingebunden (`static/vendor/uplot/`); htmx ist noch nicht eingebunden (die Oberfläche ist sonst rein serverseitig gerendert) – dafür braucht es dein Okay zum Herunterladen

Geklärt: Zielumgebung Omada (7 Geräte, im MVP etwa 70 Sensoren, SQLite reicht), SNMPv3 authNoPriv/MD5, Sprache Rust.
