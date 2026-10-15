# SNMP-Profile und Verschlüsselung

SNMP-Zugangsdaten werden als wiederverwendbare **Profile** gespeichert. Ein Gerät verweist später auf ein Profil.
Die Seiten unter `/credentials` sind nur für Administratoren zugänglich (Leser erhalten 403).

## Profiltypen

| Typ | Felder |
|---|---|
| SNMPv2c | Community |
| SNMPv3 authNoPriv | Benutzer, Authentifizierungsverfahren (MD5, SHA-1, SHA-224/256/384/512), Authentifizierungs-Passwort |
| SNMPv3 authPriv | wie authNoPriv, zusätzlich Verschlüsselungsverfahren (DES, AES-128/192/256) und Verschlüsselungs-Passwort |

Für die TP-Link-Omada-Geräte im Zielnetz gilt: **SNMPv3, authNoPriv, MD5**, Benutzer wie im Omada-Controller
(Network Config → General Settings → SNMP).

Regeln: Name 1–64 Zeichen (eindeutig, Groß-/Kleinschreibung egal), Community 1–128 Zeichen, SNMPv3-Benutzer 1–32 Zeichen ohne Leerzeichen,
SNMPv3-Passwörter 8–128 Zeichen (RFC 3414).

## Passwörter in der Oberfläche

Passwörter und Communities werden **nie** angezeigt, auch nicht verschleiert. Die Formulare zeigen nur „gesetzt – leer lassen zum Beibehalten".
Beim Bearbeiten bleiben leere Passwortfelder unverändert; ein ausgefülltes Feld ersetzt das gespeicherte Geheimnis.
Wechselt man die Version (v2c ↔ v3) oder die Stufe von authNoPriv auf authPriv, müssen die neuen Geheimnisse angegeben werden.
Beim Wechsel auf authNoPriv wird das Verschlüsselungs-Passwort gelöscht.

## Verschlüsselung in der Datenbank

- Verfahren: XChaCha20-Poly1305 (24-Byte-Zufallsnonce pro Wert). Format eines Werts: `0x01 || Nonce || Chiffrat+Tag`.
- Der Kontext (`snmp.community`, `snmp.auth_password`, `snmp.priv_password`) ist als zusätzliche authentifizierte Daten gebunden:
  ein Chiffrat lässt sich nicht in eine andere Spalte kopieren.
- Der Schlüssel (256 Bit, 64 Hex-Zeichen) liegt in einer Datei, Pfad in der Config (`secret_key_file`,
  Standard `/etc/netmon/secret.key`, Umgebungsvariable `NETMON_SECRET_KEY_FILE`).
  Der Server startet nur, wenn die Datei existiert und **nicht für Gruppe oder andere lesbar** ist (`chmod 600`).
- Schlüssel erzeugen (einmalig, überschreibt nie eine vorhandene Datei):

  ```bash
  netmon --config /etc/netmon/config.toml key generate
  ```

- **Schlüssel getrennt von der Datenbank sichern.** Ohne Schlüssel sind die gespeicherten Geheimnisse nicht mehr lesbar;
  die Profile müssten neu eingegeben werden. Ein Schlüsselwechsel (Rotation) ist noch nicht umgesetzt, das Format enthält dafür ein Versionsbyte.
- Entschlüsselt wird nur für den Poller (`storage::credentials::load_credentials`), nie für die Weboberfläche.

## Datenbank

Tabelle `credential_profiles` (Migration `0003`). Ein `CHECK`-Constraint erzwingt, dass je Version und Sicherheitsstufe genau die
passenden Spalten gefüllt sind (z. B. kein Verschlüsselungs-Passwort bei authNoPriv).
Das Löschen eines Profils, das noch von Geräten verwendet wird, wird mit der Geräte-Tabelle gesperrt (`ON DELETE RESTRICT`, folgt im nächsten Schritt).
