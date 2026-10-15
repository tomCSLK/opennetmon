# netmon

Agentloses Netzwerk-Monitoring per SNMP mit Weboberfläche, inspiriert vom Bedienkonzept von PRTG.
Ein einzelnes, statisch gelinktes Binary (Rust) mit SQLite – gedacht für den Betrieb auf einem Ubuntu Server ohne GUI, Bedienung im Browser.

> „netmon“ ist ein Arbeitstitel. „PRTG“ ist eine Marke von Paessler; dieses Projekt steht in keiner Verbindung zu Paessler.
> Zielumgebung ist ein privates Netz mit TP-Link Omada (Switches und Access Points per SNMPv3); es funktioniert mit jedem Gerät, das SNMP v2c oder v3 spricht.

## Status

Funktionierender Stand nach MVP-Schritt 4 von 6. Details und Reihenfolge: [`PLAN.md`](PLAN.md).

| | |
|---|---|
| Fertig | Login mit Rollen (Admin/Leser), SNMP-Profile (v2c, v3 authNoPriv/authPriv, verschlüsselt gespeichert), Gruppen und Geräte, Verbindungstest, Sensoren (Ping, Uptime, Interface-Traffic, Interface-Status, eigene OID), Interface-Discovery, Poller mit Statuslogik und Ereignislog, Schwellwerte, Graphen (1 h / 24 h / 7 d / 30 d), Rollups und Aufbewahrung |
| Offen | E-Mail-Benachrichtigung bei Statuswechsel, systemd-Unit, htmx-Feinschliff, Begrenzung von Anmeldeversuchen |
| Nicht geplant | SNMP-Traps, NetFlow/sFlow, verteilte Probes, Maps, SNMP SET, Omada-Controller-API |

## Funktionen

- **Geräte und Gruppen:** hierarchische Gruppen, Geräte mit eigenem Intervall, Timeout und Wiederholungen; Verbindungstest per Klick.
- **SNMP:** nur lesend. SNMPv2c sowie SNMPv3 (MD5, SHA-1/2; DES, AES) mit wiederverwendbaren Profilen. 64-Bit-Zähler (`ifXTable`) mit automatischem Fallback auf 32 Bit (`ifTable`).
- **Sensoren:** Ping, Uptime, Interface-Traffic, Interface-Status, benutzerdefinierte OID (Wert oder Zähler, mit Faktor). Interfaces werden per Walk gefunden und per Häkchen übernommen.
- **Zähler richtig gerechnet:** Überlauf und Gerätereboot werden erkannt; dann gibt es **keinen** Messwert statt eines Ausreißers.
- **Status:** `In Ordnung`, `Warnung`, `Ausfall`, `Unbekannt`, `Pausiert`. Ein einzelner Messfehler ist nur eine Warnung, erst der zweite in Folge ein Ausfall. Schwellwerte je Kanal (Warnung/Down, über/unter).
- **Graphen:** [uPlot](https://github.com/leeoniya/uPlot), lokal eingebunden (kein CDN), Anzeige in der Zeitzone des Browsers.
- **Daten:** SQLite im WAL-Modus; Rohwerte 7 Tage, Stundenwerte 1 Jahr, Tageswerte unbegrenzt (einstellbar).

## Schnellstart (Entwicklung)

Voraussetzung: aktuelle stabile Rust-Toolchain (Edition 2024).

```bash
cargo run -- --config ./dev/config.toml key generate                    # einmalig: Schlüssel für gespeicherte SNMP-Zugangsdaten
cargo run -- --config ./dev/config.toml user add admin --role admin     # ersten Administrator anlegen (Passwort wird abgefragt)
cargo run -- --config ./dev/config.toml                                 # Server starten, http://127.0.0.1:8080
```

Danach im Browser: **SNMP-Profil** anlegen → **Neues Gerät** → beim Gerät **Interfaces suchen** → Sensoren anlegen. Die Messwerte erscheinen nach dem ersten Abfrageintervall.

Tests und Qualität:

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

Die Tests brauchen kein Netz (eingebauter Fake-SNMP-Agent). Optional laufen zusätzlich Tests gegen einen lokalen `snmpd` ([`docs/dev-testagent.md`](docs/dev-testagent.md))
und manuelle `#[ignore]`-Tests gegen echte Geräte (`tests/omada_manual.rs`).

## Konfiguration

TOML-Datei (`--config`) plus Umgebungsvariablen (`NETMON_LISTEN`, `NETMON_DATABASE_PATH`, `NETMON_LOG_FILTER`, `NETMON_SECRET_KEY_FILE`).
Beispiel mit allen Schlüsseln und ihren Standardwerten:

```toml
listen = "127.0.0.1:8080"                  # TLS übernimmt ein vorgeschalteter Reverse Proxy (Caddy, nginx)
database_path = "/var/lib/netmon/netmon.db"
log_filter = "info"
secret_key_file = "/etc/netmon/secret.key" # Rechte 0600, sonst startet der Server nicht
cookie_secure = true                       # nur für Entwicklung ohne TLS auf false setzen
session_ttl_hours = 168
poller_enabled = true
poll_concurrency = 16                      # Geräte, die gleichzeitig abgefragt werden
retention_raw_days = 7
retention_hourly_days = 365                # Tageswerte bleiben unbegrenzt
```

## Release-Build für den Server

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl   # statisches Binary, keine glibc-Abhängigkeit
```

Für den Ping ohne Root-Rechte: `sudo sysctl -w net.ipv4.ping_group_range="0 2147483647"` (dauerhaft in `/etc/sysctl.d/`) oder `CAP_NET_RAW` für den Dienst.
Eine systemd-Unit mit Hardening ist geplant (`deploy/netmon.service`).

## Sicherheit

- SNMP wird ausschließlich **lesend** genutzt.
- SNMP-Zugangsdaten liegen mit XChaCha20-Poly1305 verschlüsselt in der Datenbank; der Schlüssel steht in einer separaten Datei. **Diese Datei getrennt von der Datenbank sichern** – ohne sie sind die Profile unbrauchbar.
- Passwörter mit Argon2id, Sitzungen serverseitig, CSRF-Schutz auf allen Formularen, `HttpOnly`/`SameSite=Strict`-Cookies, strenge Content-Security-Policy (keine Inline-Skripte).
- Standardmäßig nur auf `127.0.0.1`; für den Zugriff aus dem Netz einen TLS-terminierenden Reverse Proxy davorsetzen.
- Bekannte Lücke: keine Begrenzung von Anmeldeversuchen (am Reverse Proxy begrenzen).
- Geheimnisse gehören nie ins Repository, in Logs oder in HTML-Ausgaben.

## Dokumentation

| | |
|---|---|
| [`PLAN.md`](PLAN.md) | Phasen, Stand, nächste Schritte |
| [`CLAUDE.md`](CLAUDE.md) | Architektur, Konventionen, Befehle (Arbeitsanweisungen für die Entwicklung mit Claude Code) |
| [`docs/benutzer-und-login.md`](docs/benutzer-und-login.md) | Benutzer, Rollen, Sitzungen |
| [`docs/snmp-profile.md`](docs/snmp-profile.md) | SNMP-Profile und Verschlüsselung |
| [`docs/geraete-und-gruppen.md`](docs/geraete-und-gruppen.md) | Geräte, Gruppen, Verbindungstest |
| [`docs/sensoren.md`](docs/sensoren.md) | Sensortypen, Zählerlogik, Discovery |
| [`docs/poller.md`](docs/poller.md) | Abfrage, Statuslogik, Ereignislog |
| [`docs/graphen-und-schwellwerte.md`](docs/graphen-und-schwellwerte.md) | Detailseite, Graphen, Schwellwerte, Rollups |
| [`docs/decisions/0001-sprache.md`](docs/decisions/0001-sprache.md) | Entscheidung für Rust (Spike-Ergebnisse) |

## Lizenz

Noch nicht festgelegt. Enthaltene Fremdsoftware: uPlot (MIT, `static/vendor/uplot/LICENSE`); Rust-Abhängigkeiten siehe `Cargo.toml`.
