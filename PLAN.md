# Plan: `netmon` – SNMP-Monitor für Linux (Arbeitstitel „PRTG-Nachbau")

> Dieser Plan ergänzt `CLAUDE.md`. Bei Widersprüchen gilt `CLAUDE.md`.
> „PRTG" ist eine Marke von Paessler und nur ein interner Arbeitstitel.
> **Status (2026-10-15): Phase 0 (Spike) und Phase 1 (Gerüst) sind abgeschlossen. Von MVP-Schritt 1 sind Login,
> SNMP-Credential-Profile, Gruppen, Geräte und der Verbindungstest umgesetzt. **MVP-Schritt 1 ist damit komplett.**
> Als Nächstes: Sensor-Trait, Counter-Logik und Poller (MVP 2 und Phase 4).**

## Kontext

PRTG gibt es nur für Windows. Ziel ist eine Linux-native Anwendung mit ähnlichem Bedienkonzept, die Netzwerkgeräte
agentlos per SNMP überwacht. Zielumgebung: privates Netz mit TP-Link Omada (Site `MCS6`): Gateway ER605, Switches 2× SG2008 und 1× SG2008P,
Access Points 1× EAP225 und 2× EAP653 (7 Geräte). SNMP ist im Controller site-weit nur als SNMPv3 authNoPriv/MD5 aktiviert (v1/v2c aus).
Die Omada-Controller-API ist nicht Teil des MVP.
Entwickelt wird auf Zorin OS, betrieben auf einem headless Ubuntu Server; Bedienung nur per Browser.

## Leitentscheidungen (aus CLAUDE.md)

| Thema | Entscheidung |
|---|---|
| Sprache | **Rust** (Edition 2024, stable), entschieden am 2026-10-15 nach dem Spike (`docs/decisions/0001-sprache.md`) |
| Web | `axum`, `tower-http`, `askama`, htmx, uPlot (lokal vendort, per `rust-embed` ins Binary, kein CDN, kein npm) |
| SNMP | `snmp2` (Features `tokio`, `crypto-rust`); v2c und v3 (authNoPriv/authPriv); Omada-Geräte nutzen v3 authNoPriv/MD5; ausschließlich lesend |
| Ping | `surge-ping`, bevorzugt unprivilegierte ICMP-Sockets (`ping_group_range` ist auf dem Entwicklungsrechner gesetzt) |
| DB | SQLite über `sqlx`, WAL, Migrations in `migrations/` |
| Auth | Lokale Benutzer, Rollen Admin/Leser, argon2, serverseitige Sitzungen, CSRF – umgesetzt |
| Secrets | SNMP-Passwörter mit XChaCha20-Poly1305 verschlüsselt, Schlüsseldatei 0600 (`netmon key generate`) – umgesetzt |
| Betrieb | Statisches musl-Binary, systemd-Unit, Reverse Proxy (Caddy/nginx) für TLS, Standard-Bind `127.0.0.1:8080` |

## PRTG-Funktionsprinzip, das nachgebaut wird

- Objektbaum: **Gruppe → Gerät → Sensor → Kanal**. Ein Sensor ist eine Messung (z. B. Traffic Port 1:24),
  ein Kanal ein einzelner Messwert (z. B. In-/Out-Bit/s).
- Jeder Sensor wird zyklisch abgefragt (Gerät hat Polling-Intervall, Standard 60 s).
- Messwerte werden als Zeitreihe gespeichert; Graphen für 1h/24h/7d/30d.
- Status: `Up`, `Warning`, `Down`, `Unknown`, `Paused`. Sensorstatus aus Kanälen (Schwellwerte),
  Gerätestatus aus Sensoren (schlechtester Status), Gruppenstatus analog.
- Timeout oder SNMP-Fehler ergibt `Down`.
- Beim Anlegen eines Geräts: Interface-Discovery per Walk, Nutzer wählt die Sensoren aus (kein Subnetz-Scan).

## Ergebnisse Phase 0 (Spike)

- Alle drei Switches und alle drei Access Points antworten auf SNMPv3 **authNoPriv/MD5**; authPriv wird abgelehnt.
  Der ER605 antwortet nicht auf SNMP (nur Ping-Überwachung möglich).
- Switches: `ifXTable` mit 64-Bit-Countern (9 Interfaces, nicht fortlaufende Indizes wie 49153).
  Access Points: nur `ifTable` mit 32-Bit-Countern, Wrap bei 1 Gbit/s nach rund 34 Sekunden.
- `snmp2` hat keinen eingebauten Timeout, jede Anfrage braucht einen eigenen `tokio::time::timeout`.
- Lokaler `snmpd` als Testagent: v2c (nur Systemgruppe sichtbar), v3 SHA-256/AES-128 (Anleitung: `docs/dev-testagent.md`).

## Architektur

Ein Binary, ein Prozess, Module `config`, `auth`, `crypto`, `credentials`, `devices`, `snmp`, `storage`, `poller`, `sensors`, `status`, `alerting`, `web`
(Details und Regeln siehe CLAUDE.md). Vorhanden: `config`, `auth`, `crypto`, `credentials`, `devices`, `snmp`, `storage`, `web`, `error`.

- **poller:** Polls gleichmäßig über das Intervall verteilt (Jitter), globale Nebenläufigkeit per `Semaphore`,
  Timeout/Retries pro Gerät, langsame Geräte blockieren andere nie.
- **sensors:** Trait `Sensor`, eine Implementierung je Typ. Keine Sonderfälle im Poller.
- **status:** Schwellwertauswertung, Statusübergänge, Ereignislog.
- **alerting:** E-Mail (SMTP), entkoppelt über Channel/Queue; erst nach MVP-Schritten 1–5.

### Datenmodell

Vorhanden (Migrations `0001`–`0004`): `meta`, `users`, `sessions`, `credential_profiles` (v2c/v3, Secrets verschlüsselt, `CHECK` für konsistente Spalten),
`groups` (hierarchisch, höchstens 8 Ebenen), `devices` (Gruppe und Profil mit `ON DELETE RESTRICT`, Intervall/Timeout/Wiederholungen als `CHECK`).
Geplant: `sensors`, `channels`
(Einheit, Schwellwerte), `samples`, `samples_1h`, `samples_1d`, `status_events`.

Retention (konfigurierbar): Rohdaten 7 Tage, Stundenwerte 1 Jahr, Tageswerte unbegrenzt.
Counter → Rate per Differenz/Zeitdelta; bei Wrap oder Reset (z. B. `sysUpTime` kleiner) wird **kein** Wert geschrieben.
Zeitstempel intern UTC.

### Sensortypen (MVP)

Ping, SNMP Uptime, Interface-Traffic (IF-MIB, 64-Bit-HC-Counter; für Access Points 32-Bit), Interface-Status (`ifOperStatus`),
benutzerdefinierter OID-Sensor (Gauge/Counter).

## Projektstruktur (Stand)

```
netmon/
  Cargo.toml
  CLAUDE.md, PLAN.md
  src/
    main.rs           # CLI: serve (Standard), user add|passwd|list, key generate
    lib.rs
    config.rs
    error.rs
    auth/             # Rollen, Argon2, Tokens, Validierung
    crypto.rs         # SecretBox (XChaCha20-Poly1305), Schlüsseldatei
    credentials.rs    # Profil-Domäne, Secret, Validierung
    devices.rs        # Geräte-/Gruppen-Domäne, Validierung (Host, Intervall), Gruppenbaum
    snmp/             # mod.rs (SnmpClient, probe, Fehler), testagent.rs (Fake-v2c-Agent, nur Tests)
    storage/          # mod.rs (Pool, WAL, Migrations), users.rs, credentials.rs, groups.rs, devices.rs
    web/              # mod.rs (Router, State, Layout, Header, /healthz), auth.rs (Login), overview.rs (Startseite), credentials.rs, groups.rs, devices.rs (inkl. Verbindungstest), assets.rs, testutil.rs (nur Tests)
    poller/           # geplant
    sensors/          # geplant: Trait + ping, uptime, if_traffic, if_status, custom_oid
    status/           # geplant
    alerting/         # geplant
  templates/          # askama: base, app (Kopfzeile), login, home (Gruppenbaum), credentials_list/_form, group_form, device_form, device_test
  static/             # app.css (htmx, uPlot folgen)
  examples/snmp_probe.rs
  tests/              # snmp_local.rs (lokaler snmpd), omada_manual.rs (echte Hardware, #[ignore])
  migrations/         # 0001_init, 0002_users_sessions, 0003_credential_profiles, 0004_groups_devices
  dev/                # config.toml, sync.sh, sync-filter.txt (Drive-Abgleich nur CLAUDE.md und PLAN.md)
  deploy/             # geplant: netmon.service
  docs/               # decisions/0001-sprache.md, dev-testagent.md, benutzer-und-login.md, snmp-profile.md, geraete-und-gruppen.md
```

## Umsetzungsphasen

0. ✔ **Spike:** `examples/snmp_probe` gegen lokalen Agenten und Omada-Geräte; Entscheidung Rust.
1. ✔ **Gerüst:** Config (TOML + ENV), Error-Typ mit `IntoResponse`, Logging, `/healthz`, Migrations, SQLite/WAL, Graceful Shutdown.
2. **MVP 1** (in drei Commits):
   1. ✔ Benutzer, Login, Rollen, CSRF, Sicherheits-Header, CLI zum Anlegen von Benutzern.
   2. ✔ Credential-Profile (v2c, v3 authNoPriv/authPriv) mit verschlüsselten Secrets, Schlüsseldatei 0600; Formulare zeigen nur „gesetzt"; leere Passwortfelder behalten das gespeicherte Geheimnis.
   3. ✔ Geräte und Gruppen: CRUD mit serverseitiger Validierung (IP/Hostname, Intervall), hierarchische Gruppen, Profilauswahl pro Gerät (verwendetes Profil lässt sich nicht löschen), Gruppenbaum auf der Startseite, **Verbindungstest** je Gerät (`sysDescr`/`sysUpTime`, getestet gegen alle Omada-Geräte). Die Oberfläche ist serverseitig gerendert; htmx folgt, sobald das Skript lokal vendort ist.
3. **MVP 2:** `Sensor`-Trait, Sensortypen, Counter-/Ratenlogik inkl. Wrap/Reset (32 und 64 Bit).
4. **Poller:** Scheduler mit Jitter, Semaphore, Timeouts/Retries; Statuslogik und Ereignislog.
5. **MVP 3:** Interface-Discovery per Walk mit Auswahl durch den Nutzer (Namen über `ifName`/`ifDescr`).
6. **MVP 4:** Schwellwerte je Kanal, Statusübersicht, Detailseite mit uPlot-Graph (1h/24h/7d/30d), Rollups und Retention.
7. **Betrieb:** musl-Release-Build (zuletzt nach den Geräten geprüft: statisch gelinkt, 9,3 MB), `deploy/netmon.service` mit Hardening, Reverse-Proxy-Beispiel, Betriebsdoku.
8. **MVP 6:** E-Mail-Benachrichtigung bei Statuswechsel.

Nach jedem Schritt ein lauffähiger Stand und ein kleiner, thematisch abgeschlossener Commit.
**Vor jedem Commit und Push werden `CLAUDE.md` und `PLAN.md` auf den aktuellen Stand gebracht.**

## Bewusst nicht im MVP

SNMP-Traps, NetFlow/sFlow, verteilte Probes, Netzwerkkarten/Maps, SNMP SET, Mandantenfähigkeit, Omada-Controller-API,
Herstellermibs außer gezielt benötigten TP-Link-OIDs.
Zurückgestellt gegenüber früheren Ideen: CPU/RAM/Disk-Sensoren (HOST-RESOURCES/UCD), SNMP v1, Subnetz-Scan, Docker-Betrieb.

## Verifikation

- **Unit-Tests:** Counter-/Ratenberechnung (inkl. Wrap/Reset), Statuslogik und Aggregation, Config-Parsing, Downsampling.
  Vorhanden: Config, Auth, Krypto, Profil- und Geräte-Validierung, Gruppenbaum, Fehlertyp, Storage, SNMP-Client (115 Unit-/Handler-Tests).
- **Integrationstests:** gegen lokalen `snmpd` (v2c und v3), vorhanden in `tests/snmp_local.rs` (5 Tests: Probe, falsche Community, v3 mit richtigem und falschem Passwort,
  64-Bit-Counter). Später: Gerät anlegen → Discovery → Messwerte nach 1–2 Intervallen → Agent stoppen → `Down` → Agent starten → `Up`.
- **Handler-Tests:** `axum` mit `tower::ServiceExt::oneshot` (Login, Logout, CSRF, abgelaufene Sitzung, Header, `/healthz`, Profil-Seiten mit Rollenprüfung, Maskierung, HTML-Escaping); gemeinsame Helfer in `web/testutil.rs`.
- **Gerätetests** gegen echte Omada-Hardware nur manuell, nicht in `cargo test` (`tests/omada_manual.rs`, Switch 64-Bit, AP nur 32-Bit-Counter bestätigt).
- **Vor jedem Abschluss:** `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- **Packaging:** musl-Binary auf dem Ubuntu Server per systemd starten und `/healthz` prüfen.

## Sicherheit (Kurzfassung)

SNMP nur lesend; Secrets verschlüsselt in der DB (Schlüsseldatei 0600, Rechteprüfung beim Start), nie in Logs oder HTML; argon2;
`HttpOnly`/`SameSite=Strict`-Cookies, `Secure` per Config; CSRF-Schutz für alle POST-Formulare; serverseitige Validierung
von IP, OID, Intervallen; Bind nur auf `127.0.0.1`, TLS über Reverse Proxy.
Bekannte Lücke: keine Begrenzung von Anmeldeversuchen (Reverse Proxy oder später eigene Begrenzung).

## Offene Punkte (vom Nutzer zu klären)

- Endgültiger Projektname
- Ist der Server vom Internet getrennt? (Updates, E-Mail-Versand)
- SNMPv3-Passwort des Omada-Benutzers ist kurz (8 Zeichen) und stand einmal im Chat: im Controller ändern und lokal nachtragen
- ER605 (kein SNMP): nur per Ping überwachen oder später über die Omada-API?
- Schlüsseldatei (`secret.key`): Sicherung getrennt von der Datenbank einplanen; Schlüsselrotation ist noch nicht umgesetzt
- htmx und uPlot sind noch nicht eingebunden; die Skripte müssten aus dem Netz geladen und lokal vendort werden (dein Okay nötig)
