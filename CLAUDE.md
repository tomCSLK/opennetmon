# CLAUDE.md – PRTG Clone für Linux (Arbeitstitel)

> Hinweis: „PRTG" ist eine Marke von Paessler. Der Titel ist nur ein interner Arbeitstitel.
> Crate-, Binary- und Paketname: `netmon` (Platzhalter, später umbenennbar).

## Ziel

Agentloses Netzwerk-Monitoring per SNMP mit Web-Oberfläche, inspiriert vom Bedienkonzept von PRTG.
Läuft headless auf einem **Ubuntu Server ohne GUI**; Bedienung ausschließlich über den Browser.
Entwickelt wird auf Zorin OS (Desktop), betrieben auf Ubuntu Server.

Zielumgebung: Extreme Networks – EXOS (Access/Distribution) und VOSS (Core).
Access Points werden über ExtremeCloud IQ verwaltet und sind **nicht** Teil des SNMP-MVP.

## Sprachentscheidung: Rust bevorzugt, Python als Fallback

**Phase 0 (Spike, vor allem anderen):** Ein minimales Rust-CLI (`cargo run --example snmp_probe`)
fragt `sysDescr`, `sysUpTime` und `IF-MIB::ifHCInOctets` von einem echten EXOS- und einem VOSS-Gerät ab:
- SNMPv2c
- SNMPv3 authPriv (SHA-256/AES-128, zusätzlich die tatsächlich auf den Geräten konfigurierte Kombination)
- GETBULK-Walk über `ifTable`/`ifXTable`

**Fallback auf Python nur, wenn** SNMPv3 mit der auf den Geräten konfigurierten Auth/Priv-Kombination
mit `snmp2` nicht zuverlässig funktioniert und sich das nicht mit vertretbarem Aufwand lösen lässt.
Ergebnis des Spikes in `docs/decisions/0001-sprache.md` festhalten, bevor weitergebaut wird.
Nicht eigenmächtig die Sprache wechseln – bei Problemen Befund dokumentieren und nachfragen.

## Stack (Rust)

- Runtime/Web: `tokio`, `axum`, `tower-http` (Static, Compression, Trace)
- Templates/UI: `askama` (server-seitiges HTML), **htmx** für Interaktivität, **uPlot** für Graphen.
  JS/CSS lokal vendoren (kein CDN – Server steht ggf. ohne Internet), per `rust-embed` ins Binary.
  Kein npm, kein WASM-Frontend.
- SNMP: `snmp2` mit Features `tokio` und `crypto-rust` (keine OpenSSL-Abhängigkeit)
- Ping: `surge-ping`; bevorzugt unprivilegierte ICMP-Sockets (`net.ipv4.ping_group_range`), sonst `CAP_NET_RAW`
- DB: SQLite über `sqlx` (Migrations in `migrations/`, WAL-Modus)
- Sonstiges: `serde`, `toml` (Config), `clap` (CLI), `tracing` + `tracing-subscriber`,
  `thiserror` (Lib-Fehler) / `anyhow` (nur in `main`), `argon2` (Passwörter),
  `chacha20poly1305` (Verschlüsselung gespeicherter SNMP-Secrets)
- Rust Edition 2024, stabile Toolchain

Neue Abhängigkeiten nur mit kurzer Begründung im Commit; keine unmaintained Crates.

## Stack (Python-Fallback, nur nach Entscheidung in Phase 0)

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

1. Geräteverwaltung (CRUD), Gruppen, SNMP-Credentials (v2c, v3 authPriv) als wiederverwendbare Profile
2. Sensortypen: Ping, SNMP Uptime, Interface-Traffic (IF-MIB, 64-Bit-HC-Counter), Interface-Status
   (`ifOperStatus`), benutzerdefinierter OID-Sensor (Gauge/Counter)
3. Auto-Discovery der Interfaces per Walk; Nutzer wählt, welche als Sensoren angelegt werden
4. Schwellwerte je Kanal → Statusberechnung; Statusübersicht und Detailseite mit Graph (1h/24h/7d/30d)
5. Login (lokale Benutzer), Rolle Admin/Leser
6. Benachrichtigung per E-Mail (SMTP) bei Statuswechsel – erst nach 1–5

**Nicht im MVP:** SNMP-Traps, NetFlow/sFlow, verteilte Probes, Netzwerkkarten/Maps, SNMP SET,
Mandantenfähigkeit, XIQ-API, Herstellermibs außer gezielt benötigten Extreme-OIDs.

## Architektur

Ein Binary, ein Prozess, Module:
- `config` – TOML-Datei + ENV-Overrides
- `storage` – sqlx-Repository, Migrations, Rollups, Retention
- `poller` – Scheduler, verteilt Polls gleichmäßig über das Intervall (kein Burst zur vollen Minute);
  globale Nebenläufigkeit per `Semaphore`; Timeout und Retries pro Gerät; ein langsames Gerät blockiert nie andere
- `sensors` – ein Trait `Sensor` + Implementierung je Sensortyp
- `status` – Schwellwertauswertung, Statusübergänge, Ereignislog
- `alerting` – Benachrichtigungen (entkoppelt über Channel/Queue)
- `web` – axum-Router, Handler, Templates, Auth

### Messdaten

- Rohwerte in `samples` (sensor_id, channel_id, ts, value); Rollups `samples_1h`, `samples_1d` (min/avg/max)
- Retention (konfigurierbar): Rohdaten 7 Tage, Stundenwerte 1 Jahr, Tageswerte unbegrenzt
- Counter → Rate: Differenz / Zeitdelta; Counter-Wrap und Reset (z. B. Gerätereboot, `sysUpTime` kleiner) erkennen
  und dann **keinen** Wert schreiben statt eines Ausreißers
- Zeitstempel intern UTC; Anzeige in der Zeitzone des Browsers

## Sicherheit

- SNMP ausschließlich lesend
- SNMP-Community/v3-Passwörter verschlüsselt in der DB; Schlüssel aus Datei mit Rechten 0600 (Pfad in Config)
- Secrets niemals loggen, nie in HTML ausgeben (Formulare zeigen nur „gesetzt/nicht gesetzt")
- Passwörter mit argon2; Session-Cookies `HttpOnly`, `SameSite=Strict`; CSRF-Schutz für alle POST-Formulare
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
- Integrationstests gegen lokalen `snmpd` (net-snmp) als Testagent; Gerätetests gegen echte
  Extreme-Hardware nur manuell, nicht in `cargo test`
- Handler-Tests mit `axum`/`tower::ServiceExt::oneshot`

## Befehle

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo run -- --config ./dev/config.toml
cargo build --release --target x86_64-unknown-linux-musl
```

**Vor jedem Abschluss einer Aufgabe:** `cargo fmt`, `cargo clippy --all-targets -- -D warnings`
und `cargo test` müssen fehlerfrei durchlaufen.

## Offene Punkte (vom Nutzer zu klären)

- Endgültiger Projektname
- Welche SNMPv3-Auth/Priv-Kombination ist auf EXOS/VOSS konfiguriert?
- Erwartete Anzahl Geräte/Sensoren (bestimmt Polling-Nebenläufigkeit und DB-Wahl langfristig)
- Ist der Server vom Internet getrennt? (relevant für Updates und E-Mail-Versand)
