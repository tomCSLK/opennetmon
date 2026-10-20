# Betrieb auf dem Ubuntu Server

opennetmon läuft als ein einziger Dienst (`opennetmon.service`) und ist ab Werk direkt per HTTPS erreichbar (`https://<server>:8443`, siehe „HTTPS und Zugriff
aus dem Netz“); ein Reverse Proxy ist möglich, aber nicht nötig. Es gibt nur ein Binary und eine SQLite-Datenbank; es werden weder Datenbankserver noch
Laufzeitumgebungen installiert.

## Voraussetzungen

- Ubuntu Server (getestet wird die Unit mit systemd 255, Ubuntu 24.04; ab systemd 247 sollte sie laufen), Root-Zugriff per `sudo`.
- Der Server erreicht die überwachten Geräte: **UDP 161** (SNMP) und **ICMP** (Ping), bei E-Mail den SMTP-Server (meist TCP 587/465/25).
- Die Arbeitsplätze erreichen den Server auf **TCP 8443** (HTTPS, eingebaut). Ein Reverse Proxy (Caddy, nginx) ist optional.

## Installation als Debian-Paket (empfohlen)

Für Ubuntu und Debian (amd64) gibt es ein `.deb`-Paket. Es enthält dasselbe statische Binary und richtet dasselbe ein wie der Installer unten, nur unter `/usr`
statt `/usr/local`; Updates und Deinstallation laufen über `apt`. Gebaut wird es auf dem Entwicklungsrechner mit `dev/deb.sh` (braucht `cargo install cargo-deb --locked`)
oder zusammen mit dem Archiv durch `dev/release.sh` (beides landet im GitHub-Release, geprüft mit `sha256sum -c SHA256SUMS`).

```bash
dev/deb.sh                                                    # -> target/debian/opennetmon_<version>-1_amd64.deb
scp target/debian/opennetmon_*_amd64.deb user@server:/tmp/
```

Auf dem Server:

```bash
sudo apt install /tmp/opennetmon_<version>-1_amd64.deb
sudo -u opennetmon opennetmon --config /etc/opennetmon/config.toml user add admin --role admin
sudo systemctl start opennetmon
curl -sk https://127.0.0.1:8443/healthz        # -> ok
```

| Teil | Ort |
|---|---|
| Binary, Sicherungsskript | `/usr/bin/opennetmon`, `/usr/sbin/opennetmon-backup` |
| Konfiguration | `/etc/opennetmon/config.toml` (Konfigurationsdatei des Pakets: bei Updates bleibt eine geänderte Fassung erhalten, `dpkg` fragt nach), Verzeichnis `root:opennetmon 0750` (per `dpkg-statoverride`) |
| Schlüssel | `/etc/opennetmon/secret.key`, bei der ersten Installation erzeugt (0600, Besitzer `opennetmon`), nie überschrieben |
| HTTPS | `/etc/opennetmon/tls-cert.pem` und `tls-key.pem` (selbst signiert für Hostname und IP-Adressen des Servers, nie überschrieben; ersetzbar, siehe „HTTPS“) |
| Units | `/usr/lib/systemd/system/opennetmon.service` (aktiviert, bei der ersten Installation **nicht** gestartet, es gibt noch keinen Administrator), `opennetmon-backup.service`/`.timer` (aktiv) |
| Ping | `/usr/lib/sysctl.d/99-opennetmon-ping.conf`; abschalten mit einer gleichnamigen Datei in `/etc/sysctl.d/` (z. B. Link auf `/dev/null`) |
| Lizenzen, Beispiele | `/usr/share/doc/opennetmon/` (`copyright`, `THIRD-PARTY-LICENSES.txt`, `README.md`, `examples/` mit Caddy und nginx) |

Den Schlüssel wie unten beschrieben **sofort getrennt sichern**. Eigene Anpassungen der Units gehören in ein Drop-in (`sudo systemctl edit opennetmon`), nicht in die
Dateien unter `/usr/lib`, die ein Update ersetzt.

- **Update:** neues Paket mit `sudo apt install ./opennetmon_<neu>_amd64.deb` installieren; ein laufender Dienst wird neu gestartet, Konfiguration, Schlüssel und Daten bleiben.
- **Rollback:** `sudo dpkg -i opennetmon_<alt>_amd64.deb` (gleiche Einschränkung bei Migrationen wie unter „Update und Rollback“).
- **Entfernen:** `sudo apt remove opennetmon` stoppt die Dienste und entfernt die Programmdateien; Konfiguration, Schlüssel, Daten, Sicherungen und Benutzer bleiben.
  `sudo apt purge opennetmon` löscht zusätzlich `/etc/opennetmon` **samt Schlüssel und Zertifikat**; Datenbank, Sicherungen und Benutzer bleiben auch dann (endgültig löschen wie unter „Deinstallation“).

### Umstieg vom Installer auf das Paket

Konfiguration, Schlüssel und Daten werden übernommen; nur die Programmdateien des Installers müssen vorher weg, weil Units unter `/etc/systemd/system` die des Pakets verdecken:

```bash
sudo systemctl disable --now opennetmon opennetmon-backup.timer
sudo rm /etc/systemd/system/opennetmon.service /etc/systemd/system/opennetmon-backup.* /usr/local/bin/opennetmon* /usr/local/sbin/opennetmon-backup
sudo rm -r /usr/local/share/doc/opennetmon
sudo systemctl daemon-reload
sudo apt install ./opennetmon_<version>-1_amd64.deb     # vorhandene config.toml behalten (Standardantwort von dpkg)
sudo systemctl enable --now opennetmon opennetmon-backup.timer
```

Bleiben Dateien des Installers liegen, weist das Paket bei der Installation darauf hin und nennt dieselben Schritte. `enable` ist nötig, wenn das Paket schon vor dem
Aufräumen installiert war: `systemctl disable` entfernt dann auch seine Aktivierung, und `dpkg` übernimmt diese Entscheidung bei späteren Installationen.

## Installation mit dem Installer-Skript

Auf dem Entwicklungsrechner das statische Binary bauen (keine glibc-Abhängigkeit, läuft auf jedem 64-Bit-Linux):

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
scp -r target/x86_64-unknown-linux-musl/release/opennetmon deploy LICENSE THIRD-PARTY-LICENSES.txt user@server:/tmp/opennetmon-install/
```

Alternativ das fertige Archiv `opennetmon-<version>-x86_64-linux.tar.gz` samt `SHA256SUMS` aus einem GitHub-Release nehmen (Prüfung mit
`sha256sum -c SHA256SUMS`, dann entpacken); es enthält Binary, `deploy/`, `docs/` und die Lizenzdateien. Gebaut wird es mit `dev/release.sh`.

Auf dem Server:

```bash
cd /tmp/opennetmon-install
sudo ./deploy/install.sh --binary ./opennetmon
```

Der Installer ist wiederholbar und tut Folgendes:

| Schritt | Ergebnis |
|---|---|
| Benutzer | Systembenutzer `opennetmon` (ohne Login, ohne Home) |
| Binary | `/usr/local/bin/opennetmon`; die vorherige Version bleibt als `opennetmon.previous` |
| Lizenzen | `/usr/local/share/doc/opennetmon/` (`LICENSE`, `THIRD-PARTY-LICENSES.txt`) |
| Konfiguration | `/etc/opennetmon/config.toml` (aus `deploy/config.example.toml`, **nie überschrieben**), Verzeichnis `root:opennetmon 0750` |
| Schlüssel | `/etc/opennetmon/secret.key` (neu erzeugt, Modus 0600, Besitzer `opennetmon`; **nie überschrieben**) |
| HTTPS | `/etc/opennetmon/tls-cert.pem`, `tls-key.pem` (selbst signiert, Schlüssel 0600, Besitzer `opennetmon`; **nie überschrieben**) |
| Dienst | `/etc/systemd/system/opennetmon.service`, aktiviert; Daten unter `/var/lib/opennetmon` (0750, Besitzer `opennetmon`) |
| Ping | `/etc/sysctl.d/99-opennetmon-ping.conf` (erlaubt unprivilegierte ICMP-Sockets; mit `--no-sysctl` überspringen) |
| Sicherung | täglich 03:30 (`opennetmon-backup.timer`), 14 Kopien in `/var/backups/opennetmon` (mit `--no-backup` überspringen) |

Danach den **ersten Administrator** anlegen und starten:

```bash
sudo -u opennetmon /usr/local/bin/opennetmon --config /etc/opennetmon/config.toml user add admin --role admin
sudo systemctl start opennetmon
curl -sk https://127.0.0.1:8443/healthz        # -> ok
```

Wichtig: **`/etc/opennetmon/secret.key` sofort getrennt von der Datenbank sichern** (Passwortmanager, USB-Stick, anderer Rechner). Ohne den Schlüssel sind alle
gespeicherten SNMP- und SMTP-Passwörter unbrauchbar. Die automatischen Datenbank-Sicherungen enthalten den Schlüssel bewusst nicht.

## HTTPS und Zugriff aus dem Netz

opennetmon spricht HTTPS selbst. Ab Werk (Paket und Installer) lauscht es auf **allen IPv4-Adressen an Port 8443**, und zwar nur verschlüsselt. Von jedem
Rechner im Netz:

```
https://<IP-Adresse oder Name des Servers>:8443
```

- Ein Aufruf mit `http://…:8443` bekommt nur eine Weiterleitung auf `https://`; Seiten, Anmeldedaten und Cookies gehen nie unverschlüsselt über die Leitung.
  Der Server spricht TLS 1.2 und 1.3, ältere Versionen lehnt er ab.
- Firewall: Ist `ufw` aktiv, `sudo ufw allow 8443/tcp`. Weitere Ports braucht opennetmon nicht.
- Anderer Port oder andere Adresse: `listen` in der Config ändern, z. B. `listen = "0.0.0.0:9443"`, nur eine Netzwerkkarte `listen = "192.168.0.10:8443"`,
  zusätzlich IPv6 `listen = "[::]:8443"`; danach `sudo systemctl restart opennetmon`. Ports unter 1024 siehe „Port 443“.
- Der Server sieht die echte Adresse jedes Rechners; die Begrenzung der Anmeldeversuche braucht daher kein `client_ip_header` (das ist nur für einen Proxy).

### Das mitgelieferte Zertifikat

Bei der Installation erzeugt `opennetmon tls generate` ein **selbst signiertes** Zertifikat: `/etc/opennetmon/tls-cert.pem`, der Schlüssel `tls-key.pem` mit Modus 0600
und Besitzer `opennetmon`. Es gilt zehn Jahre für den Hostnamen, alle IP-Adressen des Servers (`hostname -I`) und `localhost`. Der Befehl gibt den
**SHA-256-Fingerabdruck** aus, der Dienst schreibt ihn bei jedem Start ins Log:

```bash
journalctl -u opennetmon -b | grep fingerprint
openssl x509 -in /etc/opennetmon/tls-cert.pem -noout -fingerprint -sha256 -ext subjectAltName
```

Beim ersten Aufruf warnt der Browser („Verbindung ist nicht sicher“), weil keine ihm bekannte Zertifizierungsstelle das Zertifikat unterschrieben hat. Im Browser
das Zertifikat anzeigen (Klick auf das Warn- bzw. Schlosssymbol), den Fingerabdruck mit dem obigen vergleichen und die Ausnahme bestätigen. Verschlüsselt ist die
Verbindung genauso gut wie mit einem gekauften Zertifikat; die Warnung heißt nur, dass der Browser die Identität des Servers nicht selbst prüfen kann. Wer die
Warnung ganz loswerden will, nimmt ein eigenes Zertifikat (unten).

**Neu erzeugen**, z. B. nach einer Änderung von IP-Adresse oder Hostname. Der Befehl überschreibt nie, deshalb die alten Dateien vorher beiseitelegen; je Adresse
oder Name, unter dem der Server erreichbar sein soll, ein `--name` (Hostname, `localhost` und die Loopback-Adressen kommen von selbst dazu):

```bash
sudo mv /etc/opennetmon/tls-cert.pem /etc/opennetmon/tls-cert.pem.old
sudo mv /etc/opennetmon/tls-key.pem /etc/opennetmon/tls-key.pem.old
sudo opennetmon --config /etc/opennetmon/config.toml tls generate --name 192.168.0.10 --name opennetmon.lan
sudo chown opennetmon:opennetmon /etc/opennetmon/tls-key.pem
sudo systemctl restart opennetmon
```

### Ein eigenes Zertifikat verwenden

Jedes Zertifikat im PEM-Format geht, z. B. von einer Firmen-CA, einer eigenen kleinen Zertifizierungsstelle (unten) oder von Let's Encrypt:

- **Zertifikatsdatei:** PEM, zuerst das Serverzertifikat, danach gegebenenfalls die Zwischenzertifikate (so wie `fullchain.pem`). Es muss die Namen bzw.
  IP-Adressen enthalten, unter denen die Browser den Server aufrufen (Subject Alternative Name); ein Name nur im CN genügt heutigen Browsern nicht.
- **Schlüsseldatei:** PEM ohne Passwort, PKCS#8 (`BEGIN PRIVATE KEY`), RSA (`BEGIN RSA PRIVATE KEY`) oder EC (`BEGIN EC PRIVATE KEY`); Modus **0600**, Besitzer
  `opennetmon`, sonst startet der Dienst nicht.
- **Einsetzen:** die mitgelieferten Dateien ersetzen (oder andere Pfade in `tls_cert_file`/`tls_key_file` eintragen) und neu starten:

  ```bash
  sudo install -m 0644 -o root -g root mein-zertifikat.pem /etc/opennetmon/tls-cert.pem
  sudo install -m 0600 -o opennetmon -g opennetmon mein-schluessel.pem /etc/opennetmon/tls-key.pem
  sudo systemctl restart opennetmon
  journalctl -u opennetmon -n 5          # "HTTPS only …" mit dem Fingerabdruck des neuen Zertifikats
  ```

- Ein unbrauchbares Paar (Schlüssel passt nicht, falsches Format, zu offene Rechte) meldet der Dienst beim Start mit Dateiname und Grund und startet dann nicht.
- Zertifikate werden nur beim Start gelesen: nach einer Verlängerung `sudo systemctl restart opennetmon` (angemeldete Benutzer bleiben angemeldet).

#### Eigene kleine Zertifizierungsstelle (keine Warnung mehr im Heimnetz)

Einmal eine Zertifizierungsstelle (CA) anlegen, ihr Zertifikat auf den eigenen Rechnern als vertrauenswürdig importieren und damit das Serverzertifikat
unterschreiben. Das geht auf jedem Rechner mit `openssl`. **`ca-key.pem` gut verwahren und nicht auf dem Server liegen lassen:** Wer ihn hat, kann für alle Rechner,
die der CA vertrauen, beliebige Zertifikate ausstellen.

```bash
# 1. Die CA, einmalig (zehn Jahre)
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 3650 \
  -subj "/CN=Heimnetz CA" -keyout ca-key.pem -out ca-cert.pem \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign"

# 2. Schlüssel und Zertifikat für den Server: Namen und IP-Adresse anpassen.
#    825 Tage ist die längste Laufzeit, die Apple-Geräte annehmen.
openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
  -subj "/CN=opennetmon" -keyout tls-key.pem -out tls.csr
printf 'subjectAltName = DNS:opennetmon.lan, IP:192.168.0.10\nbasicConstraints = CA:FALSE\nkeyUsage = critical, digitalSignature\nextendedKeyUsage = serverAuth\n' > tls.ext
openssl x509 -req -in tls.csr -CA ca-cert.pem -CAkey ca-key.pem -CAcreateserial -days 825 -extfile tls.ext -out tls-cert.pem
openssl verify -CAfile ca-cert.pem tls-cert.pem            # -> tls-cert.pem: OK
```

Danach `tls-cert.pem` und `tls-key.pem` wie oben einsetzen und `ca-cert.pem` auf jedem Rechner importieren, der opennetmon aufruft:

| System / Browser | Import |
|---|---|
| Windows (Edge, Chrome) | Doppelklick auf die Datei (vorher in `ca-cert.crt` umbenennen) → „Zertifikat installieren“ → „Lokaler Computer“ → Speicher „Vertrauenswürdige Stammzertifizierungsstellen“ |
| macOS (Safari, Chrome) | Schlüsselbundverwaltung → Schlüsselbund „System“ → Datei hineinziehen → Doppelklick → „Vertrauen“ → „Immer vertrauen“ |
| Firefox (alle Systeme) | Einstellungen → Datenschutz & Sicherheit → Zertifikate anzeigen → Zertifizierungsstellen → Importieren → „Dieser CA vertrauen, um Websites zu identifizieren“ |
| Chrome/Chromium unter Linux | Einstellungen → Datenschutz und Sicherheit → Sicherheit → Zertifikate verwalten → Zertifizierungsstellen → Importieren |
| Linux-Werkzeuge (`curl` …) | `sudo cp ca-cert.pem /usr/local/share/ca-certificates/heimnetz-ca.crt && sudo update-ca-certificates` |

Nach 825 Tagen Schritt 2 wiederholen; die CA und ihr Import bleiben.

#### Let's Encrypt

Geht nur mit einer eigenen öffentlichen Domain (z. B. `opennetmon.example.org`, die im LAN auf den Server zeigt). Weil der Server meist nicht aus dem Internet
erreichbar sein soll, holt man das Zertifikat über die DNS-Prüfung (certbot mit dem DNS-Plugin des Domain-Anbieters). Damit opennetmon jede Verlängerung
übernimmt, ein Deploy-Hook, z. B. `/etc/letsencrypt/renewal-hooks/deploy/opennetmon` (ausführbar; Beispiel, nicht mit opennetmon getestet):

```sh
#!/bin/sh
install -m 0644 "$RENEWED_LINEAGE/fullchain.pem" /etc/opennetmon/tls-cert.pem
install -m 0600 -o opennetmon -g opennetmon "$RENEWED_LINEAGE/privkey.pem" /etc/opennetmon/tls-key.pem
systemctl restart opennetmon
```

### Port 443

Ports unter 1024 darf der Dienst wegen der Härtung nicht öffnen; der Start bricht dann mit `binding 0.0.0.0:443 (ports below 1024 need CAP_NET_BIND_SERVICE …)` ab.
Wer `https://<server>` ohne Portangabe möchte, erlaubt genau dieses eine Recht per Drop-in:

```bash
sudo systemctl edit opennetmon
```

```ini
[Service]
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
AmbientCapabilities=CAP_NET_BIND_SERVICE
```

Dann `listen = "0.0.0.0:443"` in der Config, `sudo systemctl restart opennetmon` und gegebenenfalls `sudo ufw allow 443/tcp`.

### Bestehende Installationen umstellen

Installer und Paket überschreiben eine geänderte `config.toml` nie. Wer vor 0.3.0 installiert und die Config angepasst hat, behält deshalb `127.0.0.1:8080` ohne
HTTPS; beim Paket-Update fragt `dpkg`, Standard ist „eigene Fassung behalten“ (die neue liegt dann als `config.toml.dpkg-dist` daneben). Das Zertifikat wird
trotzdem erzeugt. HTTPS einschalten mit diesen drei Zeilen in `/etc/opennetmon/config.toml` (eine vorhandene `listen`-Zeile ersetzen, `client_ip_header` ohne Proxy
entfernen) und `sudo systemctl restart opennetmon`:

```toml
listen = "0.0.0.0:8443"
tls_cert_file = "/etc/opennetmon/tls-cert.pem"
tls_key_file = "/etc/opennetmon/tls-key.pem"
```

### Alternativ: hinter einem Reverse Proxy

Wer ohnehin Caddy oder nginx betreibt, kann opennetmon auch dahinter stellen. Dann in der Config `listen = "127.0.0.1:8080"` setzen, die beiden `tls_`-Zeilen
entfernen und `client_ip_header = "x-forwarded-for"` eintragen (Details in `docs/benutzer-und-login.md`), sonst zählt die Login-Begrenzung alle Zugriffe als eine Adresse.

- Caddy: `deploy/Caddyfile.example` (automatisches HTTPS; für ein privates Netz `tls internal`).
- nginx: `deploy/nginx.conf.example` (TLS, HSTS und eine zusätzliche **Begrenzung der Login-Versuche**: 10 pro Minute und Adresse).
- `cookie_secure = true` (Standard) setzt HTTPS voraus. Nur zum Test ohne HTTPS auf `false` stellen.
- Von außen nur 443 (und 80 für die Weiterleitung) öffnen, **nicht 8080**: `sudo ufw allow 443/tcp && sudo ufw allow 80/tcp`.

Allgemein gilt: Der Server sollte nicht aus dem Internet erreichbar sein, wenn es nicht nötig ist; für den Zugriff von unterwegs besser ein VPN ins eigene Netz.

## Konfiguration

`/etc/opennetmon/config.toml`, alle Schlüssel mit Erklärung in `deploy/config.example.toml`. Nach Änderungen `sudo systemctl restart opennetmon`.
Nur **hinter einem Reverse Proxy** zusätzlich `client_ip_header = "x-forwarded-for"` eintragen (siehe oben).
Die E-Mail-Einstellungen sind **nicht** in der Datei, sondern in der Oberfläche („E-Mail“, siehe `docs/benachrichtigung.md`) und gelten sofort.

Die Zugangsdaten der Geräte (SNMP-Profile) legst du ebenfalls in der Oberfläche an. Für die Omada-Geräte: SNMPv3, authNoPriv, MD5, Benutzer wie im
Controller unter *Network Config → General Settings → SNMP* (`docs/snmp-profile.md`).

## Alltag

```bash
systemctl status opennetmon                 # läuft der Dienst?
journalctl -u opennetmon -f                 # Log live (nie Passwörter; Statuswechsel, Fehler)
journalctl -u opennetmon --since today -p warning
sudo systemctl restart opennetmon
curl -sk https://127.0.0.1:8443/healthz # ok / unavailable (ohne interne Details)
systemctl list-timers opennetmon-backup     # nächste Sicherung
```

Beim Stoppen beendet der Dienst zuerst das Abfragen und verschickt dann noch wartende Alarm-Mails (höchstens ca. 30 s, danach beendet systemd ihn).

## Update und Rollback

Update: neues Binary bauen, wie oben kopieren, `sudo ./deploy/install.sh --binary ./opennetmon` erneut ausführen. Der Installer ersetzt Binary und Units, **lässt Konfiguration,
Schlüssel und Datenbank unberührt** und startet den laufenden Dienst neu; Datenbank-Migrationen laufen beim Start automatisch.

Vor einem Update mit Schemaänderung (neue Migration unter `migrations/`) lohnt eine Sicherung: `sudo systemctl start opennetmon-backup`.

Rollback: Migrationen gehen nur vorwärts. Ein altes Binary läuft deshalb nur sicher, wenn das Update **keine** neue Migration brachte; sonst Datenbank aus der Sicherung davor
zurückspielen (siehe unten) und das alte Binary einsetzen:

```bash
sudo systemctl stop opennetmon
sudo cp -p /usr/local/bin/opennetmon.previous /usr/local/bin/opennetmon
# bei neuer Migration zusätzlich die Datenbank aus der Sicherung vor dem Update zurückspielen
sudo systemctl start opennetmon
```

## Sicherung und Wiederherstellung

- **Automatisch:** `opennetmon-backup.timer` schreibt täglich eine konsistente Kopie (`opennetmon backup`, SQLite `VACUUM INTO`, Modus 0600) nach `/var/backups/opennetmon/opennetmon-JJJJMMTT-HHMMSS.db`
  und behält die letzten 14. Das geht im laufenden Betrieb. **Diese Dateien liegen auf demselben Server** – für echte Sicherheit regelmäßig auf einen anderen Rechner kopieren
  (z. B. `rsync -a /var/backups/opennetmon/ backup-host:opennetmon/`).
- **Manuell:** `sudo -u opennetmon /usr/local/bin/opennetmon --config /etc/opennetmon/config.toml backup /pfad/zur/neuen-datei.db` (überschreibt nie eine vorhandene Datei).
- **Schlüssel:** getrennt aufbewahren (siehe oben). Mit Datenbank-Sicherung **und** Schlüssel lässt sich alles wiederherstellen.
- **Wiederherstellen:**

  ```bash
  sudo systemctl stop opennetmon
  sudo cp -p /var/backups/opennetmon/opennetmon-20261015-033000.db /var/lib/opennetmon/opennetmon.db
  sudo rm -f /var/lib/opennetmon/opennetmon.db-wal /var/lib/opennetmon/opennetmon.db-shm
  sudo chown opennetmon:opennetmon /var/lib/opennetmon/opennetmon.db && sudo chmod 600 /var/lib/opennetmon/opennetmon.db
  sudo systemctl start opennetmon
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
Alternative ohne sysctl: in `/etc/systemd/system/opennetmon.service` die beiden Zeilen `CapabilityBoundingSet=`/`AmbientCapabilities=` durch `CAP_NET_RAW` ersetzen
(Hinweis steht in der Unit), dann `systemctl daemon-reload && systemctl restart opennetmon`.

## Die Unit im Überblick (Härtung)

Der Dienst läuft als `opennetmon` ohne Rechte (`CapabilityBoundingSet=` leer, `NoNewPrivileges`), mit schreibgeschütztem System (`ProtectSystem=strict`; schreibbar ist nur
`/var/lib/opennetmon`), ohne Home, mit privatem `/tmp` und ohne Geräte, nur mit IPv4/IPv6/Unix-Sockets, mit Syscall-Filter (`@system-service` ohne `@privileged @resources`) und
`MemoryDenyWriteExecute`. `systemd-analyze security opennetmon.service` bewertet sie mit „OK“ (Exposure 1,3). Als transienter systemd-Dienst mit denselben Optionen getestet (ohne `ProtectHome` und `PrivateTmp`, die das Test-Binary verdeckt hätten):
SNMPv3, Ping inkl. Namensauflösung, Discovery und Poller laufen ohne Blockade durch den Syscall-Filter.

## Fehlersuche

| Beobachtung | Ursache und Abhilfe |
|---|---|
| Dienst startet nicht, Log: `secret key file … must not be accessible by group or others` | Schlüsseldatei hat falsche Rechte: `sudo chmod 600 /etc/opennetmon/secret.key && sudo chown opennetmon:opennetmon /etc/opennetmon/secret.key` |
| Log: `cannot read … secret.key` | Datei fehlt oder gehört root: `sudo chown opennetmon:opennetmon …`; fehlt sie ganz, `opennetmon key generate` – dann sind alte Passwörter verloren |
| Weboberfläche: Anmeldung klappt, aber man landet wieder auf der Anmeldeseite | `cookie_secure = true`, aber Aufruf per HTTP: über HTTPS aufrufen |
| Browser: „Verbindung ist nicht sicher“ | Selbst signiertes Zertifikat: Fingerabdruck vergleichen und Ausnahme bestätigen, oder ein eigenes Zertifikat einsetzen (siehe „HTTPS“) |
| Browser: Seite nicht erreichbar | Port in der Firewall freigeben (`sudo ufw allow 8443/tcp`), Adresse mit `https://` und `:8443` aufrufen; `ss -ltnp \| grep 8443` auf dem Server |
| Dienst startet nicht, Log: `loading the HTTPS certificate …` | Datei fehlt, Schlüssel passt nicht zum Zertifikat oder hat zu offene Rechte (`chmod 600`, Besitzer `opennetmon`); neu erzeugen siehe „HTTPS“ |
| Dienst startet nicht, Log: `binding 0.0.0.0:443 …` | Port unter 1024: siehe „Port 443“ |
| Ping-Sensoren „kein ICMP-Socket verfügbar“ | siehe „Ping braucht eine Berechtigung“ |
| Alle SNMP-Sensoren eines Geräts „Keine Antwort“ | Gerät/Port nicht erreichbar, SNMP im Controller aus, falsches Profil; Verbindungstest am Gerät, `tcpdump -ni any udp port 161` |
| „Anmeldung fehlgeschlagen“ bei SNMPv3 | Benutzer, Passwort oder Verfahren (MD5/SHA) stimmen nicht |
| Keine Mail, Tabelle „Letzte Sendeversuche“ zeigt Fehler | Meldung dort lesen; Test-E-Mail senden; SMTP-Port/Verschlüsselung prüfen (587 STARTTLS, 465 TLS) |
| Ausfall-Mail, obwohl das Gerät erreichbar ist | erster Fehler ist nur „Warnung“, erst der zweite in Folge „Ausfall“; Intervall/Timeout am Gerät prüfen |
| `healthz` liefert `unavailable` | Datenbank nicht erreichbar: Platz (`df -h /var/lib/opennetmon`), Rechte, Log |

## Deinstallation

```bash
sudo systemctl disable --now opennetmon opennetmon-backup.timer
sudo rm /etc/systemd/system/opennetmon.service /etc/systemd/system/opennetmon-backup.* /usr/local/bin/opennetmon* /usr/local/sbin/opennetmon-backup /etc/sysctl.d/99-opennetmon-ping.conf
sudo rm -r /usr/local/share/doc/opennetmon
sudo systemctl daemon-reload
# Daten und Schlüssel bleiben bewusst erhalten; zum endgültigen Löschen:
# sudo rm -r /var/lib/opennetmon /etc/opennetmon /var/backups/opennetmon && sudo userdel opennetmon
```

## Checkliste vor dem Produktivbetrieb

- [ ] `secret.key` getrennt gesichert
- [ ] Aufruf per `https://<server>:8443` klappt, Fingerabdruck des Zertifikats einmal verglichen (oder eigenes Zertifikat eingesetzt)
- [ ] Port 8443 nur im LAN erreichbar, nicht aus dem Internet (Zugriff von außen per VPN)
- [ ] Nur hinter einem Reverse Proxy: `client_ip_header = "x-forwarded-for"` gesetzt und 8080 nicht von außen erreichbar
- [ ] SNMP-Passwort der Geräte lang und einzigartig (Omada: nur authNoPriv/MD5, daher nur im privaten Netz)
- [ ] Sicherungen regelmäßig auf einen anderen Rechner kopiert, eine Wiederherstellung einmal probiert
- [ ] Test-E-Mail angekommen
