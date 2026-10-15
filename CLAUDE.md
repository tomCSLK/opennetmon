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
- SNMP: `snmp2` mit Features `tokio`, `crypto-rust` (keine OpenSSL-Abhängigkeit) und **`heap_buffers`** (Pflicht: sonst liegt ein 64-KB-Puffer
  auf dem Stack und überläuft in Tokio-Threads mit 2 MB Stack)
- Ping: `surge-ping`; bevorzugt unprivilegierte ICMP-Sockets (`net.ipv4.ping_group_range`), sonst `CAP_NET_RAW`
- DB: SQLite über `sqlx` (Migrations in `migrations/`, WAL-Modus)
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
3. Auto-Discovery der Interfaces per Walk; Nutzer wählt, welche als Sensoren angelegt werden
4. Schwellwerte je Kanal → Statusberechnung; Statusübersicht und Detailseite mit Graph (1h/24h/7d/30d)
5. Login (lokale Benutzer), Rolle Admin/Leser – **umgesetzt** (Anlegen per CLI, siehe `docs/benutzer-und-login.md`)
6. Benachrichtigung per E-Mail (SMTP) bei Statuswechsel – erst nach 1–5

**Nicht im MVP:** SNMP-Traps, NetFlow/sFlow, verteilte Probes, Netzwerkkarten/Maps, SNMP SET,
Mandantenfähigkeit, Omada-Controller-API, Herstellermibs außer gezielt benötigten TP-Link-OIDs.

## Architektur

Ein Binary, ein Prozess, Module:
- `config` – TOML-Datei + ENV-Overrides (`NETMON_LISTEN`, `NETMON_DATABASE_PATH`, `NETMON_LOG_FILTER`)
- `auth` – Rollen, Argon2-Hashing, Session-/CSRF-Tokens, Validierung von Benutzername und Passwort
- `snmp` – `SnmpClient` (GET mit Timeout/Retries, v2c und v3, `heap_buffers`), `probe` für den Verbindungstest; Poller und Sensoren bauen darauf auf
- `devices` – Domäne und Validierung von Geräten und Gruppen (Host, Intervall, Gruppenbaum); `crypto` – `SecretBox` (XChaCha20-Poly1305), Schlüsseldatei mit Rechteprüfung; `credentials` – Profil-Domäne und Validierung (`Secret` mit geschwärztem `Debug`)
- `storage` – sqlx-Repository (`storage::users` für Benutzer und Sitzungen, `storage::credentials` für SNMP-Profile, `storage::groups`, `storage::devices`), Migrations, Rollups, Retention
- `poller` – Scheduler, verteilt Polls gleichmäßig über das Intervall (kein Burst zur vollen Minute);
  globale Nebenläufigkeit per `Semaphore`; Timeout und Retries pro Gerät; ein langsames Gerät blockiert nie andere
- `sensors` – Trait `Sensor` (`poll` mit `PollContext`, liefert `Measurement` aus `ChannelReading`s; `value: None` = kein Sample) + Implementierung je Sensortyp (noch keine). `sensors::counter`: `CounterTracker` berechnet Raten aus 32-/64-Bit-Countern; Reset, Neustart (`sysUpTime` kleiner), unbestätigter 32-Bit-Wrap und unplausible Raten liefern `Rate::Skipped` statt eines Werts
- `status` – Schwellwertauswertung, Statusübergänge, Ereignislog
- `alerting` – Benachrichtigungen (entkoppelt über Channel/Queue)
- `web` – axum-Router, Handler, Templates (`templates/`), eingebettete Assets (`static/`), Login (`web::auth`,
  Extractor `CurrentUser`; POST-Handler prüfen `verify_csrf`, Admin-Handler `require_admin`), Startseite als Gruppenbaum
  (`web::overview`), Gruppen-, Geräte- und Profilseiten (`web::groups`, `web::devices`, `web::credentials`)

### Messdaten

- Rohwerte in `samples` (sensor_id, channel_id, ts, value); Rollups `samples_1h`, `samples_1d` (min/avg/max)
- Retention (konfigurierbar): Rohdaten 7 Tage, Stundenwerte 1 Jahr, Tageswerte unbegrenzt
- Counter → Rate: Differenz / Zeitdelta; Counter-Wrap und Reset (z. B. Gerätereboot, `sysUpTime` kleiner) erkennen
  und dann **keinen** Wert schreiben statt eines Ausreißers
- Zeitstempel intern UTC; Anzeige in der Zeitzone des Browsers
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
- Bekannte Lücke: keine Begrenzung von Anmeldeversuchen (Reverse Proxy oder später eigene Begrenzung)
- Omada-SNMP ist nur authNoPriv/MD5 (keine Verschlüsselung, schwacher Hash): nur im privaten Netz, Passwort lang und einzigartig wählen
- Standard-Bind `127.0.0.1:8080`; TLS über vorgeschalteten Reverse Proxy (Caddy oder nginx)
- Eingaben (IP, OID, Intervalle) serverseitig validieren

## Betrieb auf Ubuntu Server

- Release-Build als statisches Binary: Target `x86_64-unknown-linux-musl`
  (vermeidet glibc-Versionskonflikte zwischen Zorin und dem Server)
- systemd-Unit unter `deploy/netmon.service`, eigener Systembenutzer `netmon`, Hardening
  (`ProtectSystem=strict`, `NoNewPrivileges=yes`, `ReadWritePaths=/var/lib/netmon`)
- Pfade: Config `/etc/netmon/config.toml`, Daten `/var/lib/netmon/`, Logs über journald
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
  ohne `NETMON_TEST_V3_*`); Gerätetests gegen echte Omada-Hardware nur manuell (`tests/omada_manual.rs`, alle `#[ignore]`)
- Handler-Tests mit `axum`/`tower::ServiceExt::oneshot`, gemeinsame Helfer in `src/web/testutil.rs`; SNMP-Tests ohne Netz über den
  Fake-v2c-Agenten `src/snmp/testagent.rs`

## Befehle

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo run -- --config ./dev/config.toml
cargo run -- --config ./dev/config.toml key generate                    # einmalig, vor dem ersten Serverstart
cargo run -- --config ./dev/config.toml user add <name> --role admin    # auch: user passwd, user list
cargo run --example snmp_probe -- -t <ip>:161 v3 -u <user> --level auth-no-priv --auth md5   # Passwörter nur per Umgebung
cargo test --test omada_manual -- --ignored --nocapture                 # manuell, braucht omada.env (siehe Datei)
cargo build --release --target x86_64-unknown-linux-musl
```

**Vor jedem Abschluss einer Aufgabe:** `cargo fmt`, `cargo clippy --all-targets -- -D warnings`
und `cargo test` müssen fehlerfrei durchlaufen.

## Offene Punkte (vom Nutzer zu klären)

- Endgültiger Projektname
- Ist der Server vom Internet getrennt? (relevant für Updates und E-Mail-Versand)
- SNMPv3-Passwort des Omada-Benutzers ist kurz (8 Zeichen) und stand einmal im Chat: im Controller auf ein längeres
  Passwort ändern und lokal in `~/.config/netmon-dev/omada.env` nachtragen (nie ins Repo oder auf Drive)
- ER605 (kein SNMP): nur per Ping überwachen oder später über die Omada-API?
- Schlüsseldatei (`secret.key`): Sicherung getrennt von der Datenbank einplanen; Schlüsselrotation ist noch nicht umgesetzt
- htmx und uPlot sind noch nicht eingebunden (die Oberfläche ist bisher rein serverseitig gerendert); die Skripte müssten aus dem Netz
  geladen und lokal vendort werden, dafür braucht es dein Okay

Geklärt: Zielumgebung Omada (7 Geräte, im MVP etwa 70 Sensoren, SQLite reicht), SNMPv3 authNoPriv/MD5, Sprache Rust.
