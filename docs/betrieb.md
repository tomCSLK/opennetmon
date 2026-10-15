# Betrieb auf dem Ubuntu Server

netmon läuft als ein einziger Dienst (`netmon.service`) hinter einem TLS-Reverse-Proxy. Es gibt nur ein Binary und eine SQLite-Datenbank; es werden weder
Datenbankserver noch Laufzeitumgebungen installiert.

## Voraussetzungen

- Ubuntu Server (getestet wird die Unit mit systemd 255, Ubuntu 24.04; ab systemd 247 sollte sie laufen), Root-Zugriff per `sudo`.
- Der Server erreicht die überwachten Geräte: **UDP 161** (SNMP) und **ICMP** (Ping), bei E-Mail den SMTP-Server (meist TCP 587/465/25).
- Ein Reverse Proxy mit TLS (Caddy oder nginx) – netmon selbst spricht nur HTTP und lauscht standardmäßig auf `127.0.0.1:8080`.

## Bauen und installieren

Auf dem Entwicklungsrechner das statische Binary bauen (keine glibc-Abhängigkeit, läuft auf jedem 64-Bit-Linux):

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
scp -r target/x86_64-unknown-linux-musl/release/netmon deploy user@server:/tmp/netmon-install/
```

Auf dem Server:

```bash
cd /tmp/netmon-install
sudo ./deploy/install.sh --binary ./netmon
```

Der Installer ist wiederholbar und tut Folgendes:

| Schritt | Ergebnis |
|---|---|
| Benutzer | Systembenutzer `netmon` (ohne Login, ohne Home) |
| Binary | `/usr/local/bin/netmon`; die vorherige Version bleibt als `netmon.previous` |
| Konfiguration | `/etc/netmon/config.toml` (aus `deploy/config.example.toml`, **nie überschrieben**), Verzeichnis `root:netmon 0750` |
| Schlüssel | `/etc/netmon/secret.key` (neu erzeugt, Modus 0600, Besitzer `netmon`; **nie überschrieben**) |
| Dienst | `/etc/systemd/system/netmon.service`, aktiviert; Daten unter `/var/lib/netmon` (legt systemd an) |
| Ping | `/etc/sysctl.d/99-netmon-ping.conf` (erlaubt unprivilegierte ICMP-Sockets; mit `--no-sysctl` überspringen) |
| Sicherung | täglich 03:30 (`netmon-backup.timer`), 14 Kopien in `/var/backups/netmon` (mit `--no-backup` überspringen) |

Danach den **ersten Administrator** anlegen und starten:

```bash
sudo -u netmon /usr/local/bin/netmon --config /etc/netmon/config.toml user add admin --role admin
sudo systemctl start netmon
curl -s http://127.0.0.1:8080/healthz          # -> ok
```

Wichtig: **`/etc/netmon/secret.key` sofort getrennt von der Datenbank sichern** (Passwortmanager, USB-Stick, anderer Rechner). Ohne den Schlüssel sind alle
gespeicherten SNMP- und SMTP-Passwörter unbrauchbar. Die automatischen Datenbank-Sicherungen enthalten den Schlüssel bewusst nicht.

## Reverse Proxy und Firewall

- Caddy: `deploy/Caddyfile.example` (automatisches HTTPS; für ein privates Netz `tls internal`).
- nginx: `deploy/nginx.conf.example` (TLS, HSTS und eine **Begrenzung der Login-Versuche**: 10 pro Minute und Adresse – netmon begrenzt Fehlversuche zusätzlich selbst, siehe `docs/benutzer-und-login.md`).
- `cookie_secure = true` (Standard) setzt voraus, dass die Seite über HTTPS aufgerufen wird. Nur zum Test ohne HTTPS auf `false` stellen.
- Von außen nur 443 (und 80 für die Weiterleitung) öffnen, **nicht 8080**. Beispiel: `sudo ufw allow 443/tcp && sudo ufw allow 80/tcp`.
- Der Server sollte nicht aus dem Internet erreichbar sein, wenn es nicht nötig ist; Zugriff besser per VPN oder nur im LAN.

## Konfiguration

`/etc/netmon/config.toml`, alle Schlüssel mit Erklärung in `deploy/config.example.toml`. Nach Änderungen `sudo systemctl restart netmon`.
**Hinter dem Reverse Proxy** `client_ip_header = "x-forwarded-for"` eintragen (Details in `docs/benutzer-und-login.md`), sonst zählt die Login-Begrenzung alle Zugriffe als eine Adresse.
Die E-Mail-Einstellungen sind **nicht** in der Datei, sondern in der Oberfläche („E-Mail“, siehe `docs/benachrichtigung.md`) und gelten sofort.

Die Zugangsdaten der Geräte (SNMP-Profile) legst du ebenfalls in der Oberfläche an. Für die Omada-Geräte: SNMPv3, authNoPriv, MD5, Benutzer wie im
Controller unter *Network Config → General Settings → SNMP* (`docs/snmp-profile.md`).

## Alltag

```bash
systemctl status netmon                 # läuft der Dienst?
journalctl -u netmon -f                 # Log live (nie Passwörter; Statuswechsel, Fehler)
journalctl -u netmon --since today -p warning
sudo systemctl restart netmon
curl -s http://127.0.0.1:8080/healthz   # ok / unavailable (ohne interne Details)
systemctl list-timers netmon-backup     # nächste Sicherung
```

Beim Stoppen beendet der Dienst zuerst das Abfragen und verschickt dann noch wartende Alarm-Mails (höchstens ca. 30 s, danach beendet systemd ihn).

## Update und Rollback

Update: neues Binary bauen, wie oben kopieren, `sudo ./deploy/install.sh --binary ./netmon` erneut ausführen. Der Installer ersetzt Binary und Units, **lässt Konfiguration,
Schlüssel und Datenbank unberührt** und startet den laufenden Dienst neu; Datenbank-Migrationen laufen beim Start automatisch.

Vor einem Update mit Schemaänderung (neue Migration unter `migrations/`) lohnt eine Sicherung: `sudo systemctl start netmon-backup`.

Rollback: Migrationen gehen nur vorwärts. Ein altes Binary läuft deshalb nur sicher, wenn das Update **keine** neue Migration brachte; sonst Datenbank aus der Sicherung davor
zurückspielen (siehe unten) und das alte Binary einsetzen:

```bash
sudo systemctl stop netmon
sudo cp -p /usr/local/bin/netmon.previous /usr/local/bin/netmon
# bei neuer Migration zusätzlich die Datenbank aus der Sicherung vor dem Update zurückspielen
sudo systemctl start netmon
```

## Sicherung und Wiederherstellung

- **Automatisch:** `netmon-backup.timer` schreibt täglich eine konsistente Kopie (`netmon backup`, SQLite `VACUUM INTO`, Modus 0600) nach `/var/backups/netmon/netmon-JJJJMMTT-HHMMSS.db`
  und behält die letzten 14. Das geht im laufenden Betrieb. **Diese Dateien liegen auf demselben Server** – für echte Sicherheit regelmäßig auf einen anderen Rechner kopieren
  (z. B. `rsync -a /var/backups/netmon/ backup-host:netmon/`).
- **Manuell:** `sudo -u netmon /usr/local/bin/netmon --config /etc/netmon/config.toml backup /pfad/zur/neuen-datei.db` (überschreibt nie eine vorhandene Datei).
- **Schlüssel:** getrennt aufbewahren (siehe oben). Mit Datenbank-Sicherung **und** Schlüssel lässt sich alles wiederherstellen.
- **Wiederherstellen:**

  ```bash
  sudo systemctl stop netmon
  sudo cp -p /var/backups/netmon/netmon-20261015-033000.db /var/lib/netmon/netmon.db
  sudo rm -f /var/lib/netmon/netmon.db-wal /var/lib/netmon/netmon.db-shm
  sudo chown netmon:netmon /var/lib/netmon/netmon.db && sudo chmod 600 /var/lib/netmon/netmon.db
  sudo systemctl start netmon
  ```

  Ohne den passenden Schlüssel bleiben Benutzer, Geräte, Sensoren und Messwerte erhalten, aber die gespeicherten Passwörter sind unlesbar: Profile und SMTP-Passwort müssen neu eingegeben werden.
  (Eine Schlüsselrotation ist noch nicht umgesetzt.)

## Datenmenge

Etwa 70 Sensoren mit 100 Kanälen bei 60-Sekunden-Intervall ergeben 144 000 Messwerte pro Tag; bei 7 Tagen Rohdatenaufbewahrung sind das rund 1 Million Zeilen,
also einige zehn Megabyte. Stunden- und Tageswerte sind vernachlässigbar klein. Die Datenbank wächst nicht über die eingestellte Aufbewahrung hinaus
(`retention_raw_days`, `retention_hourly_days`).

## Ping braucht eine Berechtigung

Ping-Sensoren nutzen unprivilegierte ICMP-Sockets. Dafür muss `net.ipv4.ping_group_range` die Gruppe des Dienstes einschließen; der Installer setzt `0 2147483647`.
Prüfen: `sysctl net.ipv4.ping_group_range`. Fehlt die Berechtigung, steht beim Start eine Warnung im Log und Ping-Sensoren melden „kein ICMP-Socket verfügbar“.
Alternative ohne sysctl: in `/etc/systemd/system/netmon.service` die beiden Zeilen `CapabilityBoundingSet=`/`AmbientCapabilities=` durch `CAP_NET_RAW` ersetzen
(Hinweis steht in der Unit), dann `systemctl daemon-reload && systemctl restart netmon`.

## Die Unit im Überblick (Härtung)

Der Dienst läuft als `netmon` ohne Rechte (`CapabilityBoundingSet=` leer, `NoNewPrivileges`), mit schreibgeschütztem System (`ProtectSystem=strict`; schreibbar ist nur
`/var/lib/netmon`), ohne Home, mit privatem `/tmp` und ohne Geräte, nur mit IPv4/IPv6/Unix-Sockets, mit Syscall-Filter (`@system-service` ohne `@privileged @resources`) und
`MemoryDenyWriteExecute`. `systemd-analyze security netmon.service` bewertet sie mit „OK“ (Exposure 1,3). Als transienter systemd-Dienst mit denselben Optionen getestet (ohne `ProtectHome` und `PrivateTmp`, die das Test-Binary verdeckt hätten):
SNMPv3, Ping inkl. Namensauflösung, Discovery und Poller laufen ohne Blockade durch den Syscall-Filter.

## Fehlersuche

| Beobachtung | Ursache und Abhilfe |
|---|---|
| Dienst startet nicht, Log: `secret key file … must not be accessible by group or others` | Schlüsseldatei hat falsche Rechte: `sudo chmod 600 /etc/netmon/secret.key && sudo chown netmon:netmon /etc/netmon/secret.key` |
| Log: `cannot read … secret.key` | Datei fehlt oder gehört root: `sudo chown netmon:netmon …`; fehlt sie ganz, `netmon key generate` – dann sind alte Passwörter verloren |
| Weboberfläche: Anmeldung klappt, aber man landet wieder auf der Anmeldeseite | `cookie_secure = true`, aber Aufruf per HTTP: über HTTPS (Proxy) aufrufen |
| Ping-Sensoren „kein ICMP-Socket verfügbar“ | siehe „Ping braucht eine Berechtigung“ |
| Alle SNMP-Sensoren eines Geräts „Keine Antwort“ | Gerät/Port nicht erreichbar, SNMP im Controller aus, falsches Profil; Verbindungstest am Gerät, `tcpdump -ni any udp port 161` |
| „Anmeldung fehlgeschlagen“ bei SNMPv3 | Benutzer, Passwort oder Verfahren (MD5/SHA) stimmen nicht |
| Keine Mail, Tabelle „Letzte Sendeversuche“ zeigt Fehler | Meldung dort lesen; Test-E-Mail senden; SMTP-Port/Verschlüsselung prüfen (587 STARTTLS, 465 TLS) |
| Ausfall-Mail, obwohl das Gerät erreichbar ist | erster Fehler ist nur „Warnung“, erst der zweite in Folge „Ausfall“; Intervall/Timeout am Gerät prüfen |
| `healthz` liefert `unavailable` | Datenbank nicht erreichbar: Platz (`df -h /var/lib/netmon`), Rechte, Log |

## Deinstallation

```bash
sudo systemctl disable --now netmon netmon-backup.timer
sudo rm /etc/systemd/system/netmon.service /etc/systemd/system/netmon-backup.* /usr/local/bin/netmon* /usr/local/sbin/netmon-backup /etc/sysctl.d/99-netmon-ping.conf
sudo systemctl daemon-reload
# Daten und Schlüssel bleiben bewusst erhalten; zum endgültigen Löschen:
# sudo rm -r /var/lib/netmon /etc/netmon /var/backups/netmon && sudo userdel netmon
```

## Checkliste vor dem Produktivbetrieb

- [ ] `secret.key` getrennt gesichert
- [ ] Reverse Proxy mit gültigem Zertifikat, 8080 nicht von außen erreichbar
- [ ] `client_ip_header = "x-forwarded-for"` in der Config gesetzt (nur hinter dem Proxy), damit die eingebaute Login-Begrenzung die echte Adresse sieht; optional zusätzlich die nginx-Begrenzung oder Zugriff nur per VPN/LAN
- [ ] SNMP-Passwort der Geräte lang und einzigartig (Omada: nur authNoPriv/MD5, daher nur im privaten Netz)
- [ ] Sicherungen regelmäßig auf einen anderen Rechner kopiert, eine Wiederherstellung einmal probiert
- [ ] Test-E-Mail angekommen
