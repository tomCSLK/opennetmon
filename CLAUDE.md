# CLAUDE.md – opennetmon

Agentloses Netzwerk-Monitoring per SNMP mit Weboberfläche, inspiriert vom Bedienkonzept von PRTG. Läuft headless auf einem **Ubuntu Server ohne GUI**
(Bedienung nur im Browser), entwickelt wird auf Zorin OS. Repo `tomCSLK/opennetmon`, **öffentlich**, Lizenz **AGPL-3.0-or-later**, Copyright Thomas Cieslik.
„PRTG“ ist eine Marke von Paessler; opennetmon ist nur vom Bedienkonzept inspiriert und steht in keiner Verbindung zu Paessler.
Name seit 2026-10-19 (vorher „netmon“, markenrechtlich geschützt). Logo seit 0.6.0 „Logo C“ (Monitor mit Pulslinie, Makro `logo` in `templates/icons.html`),
daraus `static/favicon.svg`, `static/favicon-32.png`/`-64.png` und `static/logo-mark.png`; das alte Logo (`~/Bilder/opennetmon_Logo_v1.jpeg`) steht nur noch
in `docs/assets/opennetmon-logo.jpg` (README).

## Wo steht was

| Datei | Inhalt | Laden |
|---|---|---|
| `CLAUDE.md` | Regeln und Grundwissen für jede Sitzung | automatisch |
| `CLAUDE.local.md` | Heimnetz, Instanzen, private offene Punkte (gitignored) | automatisch |
| `ARCHITEKTUR.md` | Stack mit Begründungen, alle Module im Detail, Messdaten, Sicherheit im Detail, Paket und Betrieb, Releases/README, Inventar-Endpunkt | **vor Arbeit an einem Modul den Abschnitt lesen**, danach dort nachziehen |
| `PLAN.md` | Stand, Releases, Vorhaben A–Q, offene Punkte, Verlauf | bei Bedarf |
| `docs/*.md`, `README.md` | Anwenderdoku (Englisch), Entscheidung zur Sprache in `docs/decisions/0001-language.md` | bei Bedarf |

**Diese Datei kurz halten (unter 20 000 Zeichen).** Einzelheiten zu Modulen gehören nach `ARCHITEKTUR.md`, Stand und Verlauf nach `PLAN.md`; hier nur, was in
jeder Sitzung gebraucht wird. Die ausführliche Fassung bis 2026-11-09 steht in der Git-Historie.

## Ziel und Umgebung

- Zielnetz des Nutzers: TP-Link Omada (Software-Controller) mit Gateway ER605, drei Smart-Switches (2× SG2008, 1× SG2008P) und drei Access Points
  (1× EAP225, 2× EAP653); davor FRITZ!Box und Huawei-5G-Router (eigene Sensortypen, Vorhaben N). SNMP ist im Controller site-weit nur als
  **SNMPv3 authNoPriv mit MD5** aktiv. Switches liefern 64-Bit-Counter (`ifXTable`), Access Points nur 32-Bit (`ifTable`); Interface-Indizes sind
  nicht fortlaufend (z. B. 49153), Namen über `ifName`/`ifDescr`.
- Sprache **Rust**, entschieden am 2026-10-15 nach einem Spike. Nicht eigenmächtig wechseln; bei grundlegenden Problemen Befund dokumentieren und nachfragen.
- Nicht im Umfang: SNMP-Traps, NetFlow/sFlow, verteilte Probes, Netzwerkkarten, SNMP SET, Mandanten, Omada-Controller-API.
- Begriffe: **Gerät** (Host mit SNMP-Profil oder nur Ping, Gruppe, Intervall), **Sensor** (eine Messung an einem Gerät), **Kanal** (ein Messwert mit Einheit
  und Schwellwerten), **Status** `Up`/`Warning`/`Down`/`Unknown`/`Paused` (Sensor aus Kanälen, Gerät aus Sensoren), **Gruppe** (hierarchisch).

## Technik in Kürze (Einzelheiten `ARCHITEKTUR.md`, „Stack“)

- Ein Binary, ein Prozess: `tokio`, `axum`, `tower-http`; Seiten serverseitig mit `askama`; Graphen mit uPlot; sonst wenige eigene Skripte in `static/`.
  Kein npm, kein WASM, kein htmx, kein CDN: JS, CSS und Schriften lokal, per `rust-embed` (Feature `mime-guess`) ins Binary.
- SQLite über `sqlx` (WAL, Migrationen in `migrations/`, zurzeit `0001`–`0025`; Feature `macros` für `migrate!`, keine Compile-Time-Queries).
- SNMP über `snmp2` mit den Features `tokio`, `crypto-rust` und **`heap_buffers`** (Pflicht, sonst läuft der Stack in Tokio-Threads über). `snmp2` hat
  keinen eingebauten Timeout: jede Anfrage mit `tokio::time::timeout` begrenzen.
- HTTPS eingebaut (`tokio-rustls`, `rcgen`), Mail mit `lettre` über rustls, Übersetzungen mit Fluent, Zeitzonen mit `jiff`. Kein OpenSSL, damit der
  musl-Build statisch bleibt. Rust Edition 2024, stabile Toolchain.
- **Neue Abhängigkeiten** nur mit kurzer Begründung im Commit; keine unmaintained Crates; Lizenz verträglich mit AGPL-3.0 (MIT, Apache-2.0, BSD, ISC,
  Zlib, MPL-2.0 u. Ä.; nicht GPL-2.0-only, nicht proprietär). Nach jeder Änderung an `Cargo.lock` `python3 -I dev/third-party-licenses.py` laufen lassen
  und `THIRD-PARTY-LICENSES.txt` mitcommitten.

## Module (Überblick; Einzelheiten `ARCHITEKTUR.md`, „Architektur“)

| Bereich | Module |
|---|---|
| Start und Querschnitt | `main`, `config` (TOML + `OPENNETMON_*`), `tls`, `crypto` (`SecretBox`), `credentials`, `auth` (Rollen, Argon2, Tokens, `throttle`), `i18n`, `error` |
| SNMP und Sensoren | `snmp` (Client, `probe`, `identity`, `testagent`), `discovery`, `mib` (Lexer, Parser, Index, Katalog), `value_discovery`, `sensors` (Trait `Sensor`: snmp, ping, local, service, dns, fritzbox, huawei; `counter`, `spec`), `http_client` |
| Ablauf | `poller` (eine Aufgabe je Gerät, Intervall je Sensor), `status` (Schwellwerte, Zustandsautomat), `alerting` und `mail`, `maintenance` (Rollups, Aufbewahrung), `update_check` |
| Fachlogik | `devices`, `templates` (Gerätevorlagen), `pause` (Pausen und Abhängigkeiten), `schedule` (Wartungsfenster), `availability`, `audit`, `ordering` |
| Daten | `storage::*` (je Bereich ein Modul), `migrations/` |
| Web | `web::*` (Handler, Extractor `CurrentUser`, `testutil`), `templates/` (askama), `static/`, `locales/` |

Regeln, die überall gelten:

- Neue Sensortypen nur über den Trait `Sensor`, keine Sonderfälle im Poller.
- POST-Handler prüfen `verify_csrf`, Admin-Handler `require_admin`; jede Änderung durch einen Admin ruft danach `audit::record` auf. Secrets nie als Wert ins
  Änderungsprotokoll (nur „geändert“), nie loggen, nie ins HTML (Formulare zeigen nur „gesetzt/nicht gesetzt“).
- Die Tabelle `devices` wird vom Programm einmal neu aufgebaut (`storage::migrate`); neue Gerätefelder deshalb in eine eigene Tabelle (Beispiel `device_identity`).
- Keine Inline-Styles (CSP), Breiten z. B. als SVG-Attribute. Graphen lesen ihre Farben aus den CSS-Variablen und zeichnen beim Event `opennetmon:theme` neu.
- Handy-Tabellen (`table.m-cards` + Layoutklasse): jede Zellklasse braucht einen Bereich im Layout, sonst entsteht eine schmale Zusatzspalte.
- Messdaten: Zeitstempel intern UTC in ms, Anzeige in der Zeitzone des Browsers (`web::time_html`, `static/time.js`), E-Mails in UTC. Counter → Rate:
  bei Wrap, Reset oder Neustart (`sysUpTime` kleiner) **keinen** Wert schreiben statt eines Ausreißers. Aufbewahrung: Rohwerte 7 Tage, Stundenwerte 1 Jahr,
  Tageswerte unbegrenzt (konfigurierbar).

## Sicherheit (Einzelheiten `ARCHITEKTUR.md`, „Sicherheit“)

- SNMP ausschließlich lesend. Eingaben (IP, OID, Intervalle) serverseitig validieren; OIDs höchstens 128 Stellen (`MAX_OID_ARCS`).
- SNMP- und SMTP-Secrets verschlüsselt in der DB (XChaCha20-Poly1305, Spaltenkontext als AAD); Schlüssel aus `secret_key_file` mit Rechten 0600, sonst
  startet der Server nicht (`opennetmon key generate`). Profil-Seiten nur für Admins.
- Passwörter mit Argon2, geprüft nur über `AppState::check_password` (höchstens 4 gleichzeitig). Sitzungen serverseitig, in der DB nur der SHA-256 des
  Tokens; Cookies `HttpOnly`, `SameSite=Strict`, `Secure` laut Config. CSRF-Schutz für alle POST-Formulare. Kein Standardbenutzer, erster Admin per CLI.
  Abmelden ohne Aktivität (`session_idle_minutes`; automatisches Aktualisieren zählt nicht).
- Anmeldeversuche gebremst (`auth::throttle`, je Name und Adresse, jeder Versuch zählt schon vor der Prüfung, nie dauerhaft sperren). API-Token
  (`/api/v1/inventory`) mit eigener Drossel, in der DB nur als SHA-256.
- Sicherheits-Header auf allen Antworten (CSP mit `base-uri 'self'` und `object-src 'none'`, `X-Frame-Options`, `nosniff`, `Referrer-Policy`,
  `Cache-Control: no-store` außer statischen Dateien). Fehlerseiten im Design, nie axums englische Texte.
- Omada-SNMP ist nur authNoPriv/MD5: nur im privaten Netz, Passwort lang und einzigartig.
- Ausgelieferte Config: `0.0.0.0:8443` mit eingebautem HTTPS (selbst signiertes Zertifikat bei der Installation); ohne `tls_*` `127.0.0.1:8080` hinter
  einem Reverse Proxy; kein HSTS.

## Konventionen

- Code, Bezeichner, Commit-Messages und Code-Kommentare: Englisch. `docs/`, `mibs/README.md`, `README.md` und Release-Notes: **Englisch**.
  Deutsch bleiben nur `CLAUDE.md`, `ARCHITEKTUR.md` und `PLAN.md`.
- UI-Texte nur über die Fluent-Kataloge `locales/<code>/*.ftl`; Deutsch ist Ausgangssprache, Englisch wird mitgepflegt. Seiten nutzen `t.get("id")`,
  Fachlogik liefert `Msg`, JS nutzt `T('key')`; keine festen Texte in Templates oder Rust-Code. `Msg::Text` nur für fremde Texte (Gerät, SMTP-Server).
  Benachrichtigungen in der Sprache von `mail_settings.language`. Tests prüfen die Kataloge gegeneinander und gegen Code, Templates und JS (`docs/translations.md`).
- Kein `unwrap()`/`expect()` außerhalb von Tests und Startcode; Fehler über eigenen Error-Typ mit `IntoResponse`.
- Kleine, thematisch abgeschlossene Commits, jeder Stand lauffähig. Commits mit `git commit -s` (DCO, `Signed-off-by`; die CI prüft es bei Pull Requests).
- AGPL Abschnitt 13: Benutzermenü (`templates/app.html`) und Anmeldeseite verlinken den Quellcode (`web::SOURCE_URL`) mit Version (`web::VERSION`); diese
  Links nicht entfernen. Der Installer legt `LICENSE` und `THIRD-PARTY-LICENSES.txt` nach
  `/usr/local/share/doc/opennetmon/`, das Paket `copyright` (aus `LICENSE`) und `THIRD-PARTY-LICENSES.txt` nach `/usr/share/doc/opennetmon/`.
  Lizenzangaben in `LICENSE` und `Cargo.toml` (`license`, `repository`, `authors`).
- Heimnetz-Angaben (Site, Adressen, SNMP-Benutzer) nur in `CLAUDE.local.md` und `~/.config/opennetmon-dev/omada.env`, nie ins Repo oder auf Drive;
  Tests nutzen neutrale Werte (`192.0.2.x`, Benutzer `monitor`).

## Tests

- **Vor jedem Abschluss einer Aufgabe:** `cargo fmt`, `cargo clippy --all-targets -- -D warnings` und `cargo test` müssen fehlerfrei durchlaufen.
- CI (`.github/workflows/ci.yml`): fmt, clippy, Tests, `third-party-licenses.py --check`, `cargo audit` (bei Push/PR und montags), DCO bei Pull Requests.
  Actions nur von `actions/*`, auf Commit-Hash gepinnt. Beitragsregeln in `CONTRIBUTING.md`, Meldeweg für Lücken in `SECURITY.md`.
- Unit-Tests für Counter/Raten (inkl. Wrap/Reset), Statuslogik und Config; Handler-Tests mit `tower::ServiceExt::oneshot` und den Helfern aus
  `src/web/testutil.rs`; SNMP ohne Netz über den Fake-v2c-Agenten `src/snmp/testagent.rs`.
- Gegen einen lokalen `snmpd`: `tests/snmp_local.rs` (überspringt sich ohne Agent oder ohne `OPENNETMON_TEST_V3_*`). Echte Omada-Geräte nur manuell:
  `tests/omada_manual.rs` (alle `#[ignore]`, braucht `omada.env`; u. a. `sensors_work_on_switch_and_access_point`, `poller_cycle_against_real_devices`,
  `interface_discovery_on_switch_and_access_point`).

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

## Releases und Betrieb

- Releases **nur auf Ansage**: Version in `Cargo.toml` erhöhen und committen, dann `dev/release.sh --publish --notes <datei>` (Notes auf Englisch).
  Test-Pakete ohne Tag mit `dev/deb.sh`. Ablauf im Detail in `ARCHITEKTUR.md`, „Releases und README“; Liste aller Releases in `PLAN.md`.
- Produktiv läuft opennetmon als `.deb` auf dem Server des Nutzers; Updates und Betrieb dort aus `~/Claude/Projekte/Serveradmin` (Abschnitt 4.3), nicht aus
  diesem Repo. Paket, Units, Pfade und Sicherung: `ARCHITEKTUR.md`, „Betrieb auf Ubuntu Server“; Anleitung für Anwender `docs/operations.md`.
- `README.md` (GitHub-Startseite) nur auf Englisch; bei Meilensteinen Status-Tabelle, Konfigurationsbeispiel und Doku-Liste mitpflegen.

## Stand und offene Punkte

Maßgeblich ist `PLAN.md` (oben „Stand“, Vorhaben A–Q); private Punkte in `CLAUDE.local.md`. Kurz (2026-11-12):

- **v0.5.9** released (2026-11-08); auf dem Server läuft **Test-Paket 0.6.1** (kein Tag), gebaut ist **Test-Paket 0.6.2** (2026-11-12, kein Tag,
  Korrekturen aus dem Test, `PLAN.md` P Schritt 6); die Installation macht der Nutzer.
- **Vorhaben P** (neue Oberfläche nach opensecmon: Kopfleiste, Benutzermenü mit Einstellungen, Abzeichen „Neue Version“, Suche, „Braucht Aufmerksamkeit“)
  ist umgesetzt im **Test-Paket 0.6.0** (2026-11-09, kein Tag). Beschreibung in `ARCHITEKTUR.md` („Design“), Abweichungen vom Entwurf in `PLAN.md` (P).
  Der Entwurf bleibt **lokal** (`/docs/entwurf/` in `.gitignore`). Laut Nutzer (2026-11-10) wichen Grafiken und Graphen vom Entwurf ab; Liste und
  Schrittplan in `PLAN.md` (P), Schritte 1–5 erledigt und im **Test-Paket 0.6.1** (`target/debian/opennetmon_0.6.1-1_amd64.deb`). Die Installation auf
  dem Server macht der Nutzer, danach Test und Rückmeldung; Release (Schritt 7) nur auf Ansage.
- **Vorhaben Q** (notiert 2026-11-10): Docker-Version (Image für amd64/arm64) zusätzlich zum `.deb`, Umsetzung nach „go“.
- Später: Push (F), Zwei-Faktor-Anmeldung (K), Rotation von `secret.key` (L; bis dahin Schlüsseldatei getrennt von der Datenbank sichern), Anmeldeprofile
  für die FRITZ!Box (N), apt-Repository.
- Repo: öffentlich seit 2026-10-22, mit Private vulnerability reporting, Dependabot, Secret Scanning und Push Protection. Alte Commits enthalten Site-Name,
  SNMP-Benutzer und private Adressen (wie vereinbart nicht umgeschrieben). Wegen DCO bräuchte eine spätere Doppellizenzierung die Zustimmung der Beitragenden.
