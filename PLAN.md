# Plan: `netmon` – SNMP-Monitor für Linux (Arbeitstitel „PRTG-Nachbau")

> Dieser Plan ergänzt `CLAUDE.md`. Bei Widersprüchen gilt `CLAUDE.md`.
> „PRTG" ist eine Marke von Paessler und nur ein interner Arbeitstitel.
> **Status: Vorbereitungsphase. Es wird noch kein Code geschrieben.**

## Kontext

PRTG gibt es nur für Windows. Ziel ist eine Linux-native Anwendung mit ähnlichem Bedienkonzept, die Netzwerkgeräte
agentlos per SNMP überwacht. Zielumgebung: privates Netz mit TP-Link Omada (Site `MCS6`): Gateway ER605, Switches 2× SG2008 und 1× SG2008P,
Access Points 1× EAP225 und 2× EAP653. SNMP ist im Controller site-weit nur als SNMPv3 aktiviert (v1/v2c aus).
Die Omada-Controller-API ist nicht Teil des MVP.
Entwickelt wird auf Zorin OS, betrieben auf einem headless Ubuntu Server; Bedienung nur per Browser.

## Leitentscheidungen (aus CLAUDE.md)

| Thema | Entscheidung |
|---|---|
| Sprache | Rust (Edition 2024, stable). Python/FastAPI nur als Fallback, falls der Phase-0-Spike scheitert |
| Web | `axum`, `tower-http`, `askama`, htmx, uPlot (lokal vendort, per `rust-embed` ins Binary, kein CDN, kein npm) |
| SNMP | `snmp2` (Features `tokio`, `crypto-rust`); v2c und v3 (authNoPriv/authPriv); Omada-Geräte nutzen v3 authNoPriv/MD5; ausschließlich lesend |
| Ping | `surge-ping`, bevorzugt unprivilegierte ICMP-Sockets |
| DB | SQLite über `sqlx`, WAL, Migrations in `migrations/` |
| Auth | Lokale Benutzer, Rollen Admin/Leser, argon2, Session-Cookies, CSRF |
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

## Architektur

Ein Binary, ein Prozess, Module `config`, `storage`, `poller`, `sensors`, `status`, `alerting`, `web`
(Details und Regeln siehe CLAUDE.md).

- **poller:** Polls gleichmäßig über das Intervall verteilt (Jitter), globale Nebenläufigkeit per `Semaphore`,
  Timeout/Retries pro Gerät, langsame Geräte blockieren andere nie.
- **sensors:** Trait `Sensor`, eine Implementierung je Typ. Keine Sonderfälle im Poller.
- **status:** Schwellwertauswertung, Statusübergänge, Ereignislog.
- **alerting:** E-Mail (SMTP), entkoppelt über Channel/Queue; erst nach MVP-Schritten 1–5.

### Datenmodell (Entwurf)

`groups` (hierarchisch), `credential_profiles` (v2c/v3, Secrets verschlüsselt), `devices`, `sensors`, `channels`
(Einheit, Schwellwerte), `samples`, `samples_1h`, `samples_1d`, `status_events`, `users`, `sessions`.

Retention (konfigurierbar): Rohdaten 7 Tage, Stundenwerte 1 Jahr, Tageswerte unbegrenzt.
Counter → Rate per Differenz/Zeitdelta; bei Wrap oder Reset (z. B. `sysUpTime` kleiner) wird **kein** Wert geschrieben.
Zeitstempel intern UTC.

### Sensortypen (MVP)

Ping, SNMP Uptime, Interface-Traffic (IF-MIB, 64-Bit-HC-Counter), Interface-Status (`ifOperStatus`),
benutzerdefinierter OID-Sensor (Gauge/Counter).

## Geplante Projektstruktur

```
netmon/
  Cargo.toml
  CLAUDE.md, PLAN.md
  src/
    main.rs
    config.rs
    error.rs
    storage/
    poller/
    sensors/          # mod.rs (Trait) + ping, uptime, if_traffic, if_status, custom_oid
    status/
    alerting/
    web/              # router, handlers, auth, templates/, static/ (htmx, uPlot)
  examples/snmp_probe.rs
  migrations/
  dev/config.toml
  deploy/netmon.service
  docs/
    decisions/0001-sprache.md
  tests/
```

## Vorbereitung (aktueller Schritt, ohne Code)

1. **Offene Punkte klären** (siehe unten), v. a. SNMPv3-Kombination der Omada-Geräte.
2. **Entwicklungsumgebung prüfen:** Rust-Toolchain (stable, Edition 2024), Target `x86_64-unknown-linux-musl`,
   `net-snmp` (`snmpd`, `snmpwalk`) als lokaler Testagent, `sqlite3`, `ping_group_range` für unprivilegiertes ICMP.
3. **Gerätezugang für Phase 0:** mindestens ein Omada-Switch (zusätzlich Gateway und Access Point, falls sie SNMP bieten) mit erreichbarer IP, SNMP-Zugangsdaten
   (v2c-Community, v3-Benutzer inkl. Auth/Priv-Protokolle) und Leserechten auf IF-MIB. Zugangsdaten nur lokal
   halten, nie ins Repo.
4. **Repository vorbereiten:** `git init`, `.gitignore` (u. a. `target/`, `dev/*.local.toml`, Schlüsseldateien),
   Verzeichnis `docs/decisions/` anlegen.
5. **Doku-Gerüst (Deutsch):** `docs/` mit Architekturübersicht und Betriebsanleitung als Platzhalter.
6. **Crate-Auswahl prüfen:** Aktuelle Versionen und Wartungsstatus der Abhängigkeiten aus CLAUDE.md
   verifizieren (v. a. `snmp2` mit `crypto-rust` und SHA-256/AES-128 für v3).

## Umsetzungsphasen (nach Freigabe)

0. **Spike:** `examples/snmp_probe` – `sysDescr`, `sysUpTime`, `ifHCInOctets` per v2c und v3 (authNoPriv und authPriv)
   (SHA-256/AES-128 plus die real konfigurierte Kombination), GETBULK-Walk über `ifTable`/`ifXTable`.
   Ergebnis in `docs/decisions/0001-sprache.md`; bei Problemen dokumentieren und nachfragen, nicht eigenmächtig die Sprache wechseln.
1. **Gerüst:** Config (TOML + ENV), Error-Typ mit `IntoResponse`, Logging, `/healthz`, Migrations, SQLite/WAL.
2. **MVP 1:** Geräteverwaltung (CRUD), Gruppen, Credential-Profile mit verschlüsselten Secrets; Login und Rollen.
3. **MVP 2:** `Sensor`-Trait, Sensortypen, Counter-/Ratenlogik inkl. Wrap/Reset.
4. **Poller:** Scheduler mit Jitter, Semaphore, Timeouts/Retries; Statuslogik und Ereignislog.
5. **MVP 3:** Interface-Discovery per Walk mit Auswahl durch den Nutzer.
6. **MVP 4:** Schwellwerte je Kanal, Statusübersicht, Detailseite mit uPlot-Graph (1h/24h/7d/30d), Rollups und Retention.
7. **Betrieb:** musl-Release-Build, `deploy/netmon.service` mit Hardening, Reverse-Proxy-Beispiel, Betriebsdoku.
8. **MVP 6:** E-Mail-Benachrichtigung bei Statuswechsel.

Nach jedem Schritt ein lauffähiger Stand und ein kleiner, thematisch abgeschlossener Commit.

## Bewusst nicht im MVP

SNMP-Traps, NetFlow/sFlow, verteilte Probes, Netzwerkkarten/Maps, SNMP SET, Mandantenfähigkeit, Omada-Controller-API,
Herstellermibs außer gezielt benötigten TP-Link-OIDs.
Zurückgestellt gegenüber früheren Ideen: CPU/RAM/Disk-Sensoren (HOST-RESOURCES/UCD), SNMP v1, Subnetz-Scan, Docker-Betrieb.

## Verifikation

- **Unit-Tests:** Counter-/Ratenberechnung (inkl. Wrap/Reset), Statuslogik und Aggregation, Config-Parsing, Downsampling.
- **Integrationstests:** gegen lokalen `snmpd` (v2c und v3): Gerät anlegen → Discovery → Messwerte nach 1–2 Intervallen →
  Agent stoppen → `Down` → Agent starten → `Up`.
- **Handler-Tests:** `axum` mit `tower::ServiceExt::oneshot`.
- **Gerätetests** gegen echte Omada-Hardware nur manuell, nicht in `cargo test`.
- **Vor jedem Abschluss:** `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- **Packaging:** musl-Binary auf dem Ubuntu Server per systemd starten und `/healthz` prüfen.

## Sicherheit (Kurzfassung)

SNMP nur lesend; Secrets verschlüsselt in der DB (Schlüsseldatei 0600), nie in Logs oder HTML; argon2;
`HttpOnly`/`SameSite=Strict`-Cookies; CSRF-Schutz für alle POST-Formulare; serverseitige Validierung von IP, OID, Intervallen;
Bind nur auf `127.0.0.1`, TLS über Reverse Proxy.

## Offene Punkte (vom Nutzer zu klären)

- Endgültiger Projektname
- Welche SNMPv3-Auth/Priv-Kombination nutzen die Omada-Geräte (Controller zeigt nur Benutzer `MCS6_SNMP` und Passwort)?
- Welche Geräte (Switches, ER605, EAPs) antworten tatsächlich auf SNMP?
- Erwartete Anzahl Geräte/Sensoren (Polling-Nebenläufigkeit, DB-Wahl langfristig)
- Ist der Server vom Internet getrennt? (Updates, E-Mail-Versand)
- SNMPv3-Passwort als lokale Umgebungsvariable für den Spike (nie in Chat, Repo oder Drive)
