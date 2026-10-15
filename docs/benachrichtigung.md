# E-Mail-Benachrichtigung

Bei einem Statuswechsel eines Sensors schickt netmon eine E-Mail. Die Einstellungen stehen **in der Oberfläche** unter **E-Mail** (nur Administratoren,
`/settings/mail`) und gelten sofort – ein Neustart ist nicht nötig.

## Einstellungen

| Feld | Bedeutung |
|---|---|
| Benachrichtigungen aktiv | Hauptschalter. Aus: es wird nichts verschickt (Wechsel werden weiter protokolliert). |
| Auch bei Warnungen | Aus: nur Ausfall und Wiederherstellung nach einem Ausfall werden gemeldet. |
| Server, Port | SMTP-Server (Hostname oder IP). Üblich: STARTTLS 587, TLS (SMTPS) 465, unverschlüsselt 25. |
| Verschlüsselung | STARTTLS, TLS (SMTPS) oder keine (nur für ein vertrauenswürdiges lokales Relay). |
| Benutzername, Passwort | Optional. Leerer Benutzername = ohne Anmeldung. Das Passwort wird **nie angezeigt**; leer lassen behält das gespeicherte. Gespeichert wird es wie die SNMP-Geheimnisse mit XChaCha20-Poly1305 (Kontext `smtp.password`). |
| Absender, Empfänger | Eine Absenderadresse; ein bis 20 Empfänger (eine Adresse pro Zeile). Nur reine Adressen, keine Anzeigenamen. |

Bei ausgeschalteter Benachrichtigung dürfen die Einstellungen unvollständig gespeichert werden; eingeschaltet brauchen sie Server, Absender und mindestens einen Empfänger.
Ungültige Eingaben werden auf der Seite erklärt, die Eingaben bleiben stehen (das Passwort nie).

**Test-E-Mail senden** verschickt eine Testnachricht mit den *gespeicherten* Einstellungen (erst speichern, dann testen) – auch wenn die Benachrichtigung noch aus ist.
Das Ergebnis erscheint sofort; Fehler (Server nicht erreichbar, Anmeldung abgelehnt, Empfänger abgelehnt, Zeitüberschreitung) werden im Klartext gezeigt.
Die Tabelle „Letzte Sendeversuche“ listet die letzten 20 Versuche (Alarm oder Test, Ergebnis, Fehlertext); Einträge werden nach `retention_hourly_days` gelöscht.

## Was wird gemeldet?

| Wechsel | Mail |
|---|---|
| beliebig → `Ausfall` (auch beim allerersten Wert eines Sensors) | ja |
| beliebig → `Warnung` | ja, wenn „Auch bei Warnungen“ an ist |
| `Ausfall` → `In Ordnung` | ja („Wieder in Ordnung“) |
| `Warnung` → `In Ordnung` | ja, wenn „Auch bei Warnungen“ an ist |
| erster Wert `In Ordnung` (Sensor neu, Server gestartet) | nein |
| → `Unbekannt` / `Pausiert` | nein |

Messfehler sind zunächst nur eine Warnung und erst der zweite in Folge ein Ausfall (siehe `docs/poller.md`); Schwellwertverletzungen wirken sofort.

## Zusammenfassen und Zustellen (`src/alerting.rs`)

- Der Poller legt jeden Statuswechsel in eine Warteschlange (1000 Plätze, nie blockierend; ist sie voll, wird gewarnt und der Alarm verworfen).
- Eine eigene Aufgabe sammelt Wechsel, die innerhalb von **10 Sekunden** nach dem ersten eintreffen, zu **einer Mail** („3 Statuswechsel: 2 Ausfall, 1 Warnung“).
  Fällt ein ganzes Gerät aus, kommt so eine Mail statt eine je Sensor. Ein einzelner Wechsel hat einen eigenen Betreff, z. B. `[netmon] Ausfall: Switch Büro / Ping`.
- Die Einstellungen werden für jede Mail frisch gelesen. Beim Beenden des Servers wird eine noch wartende Sammlung noch verschickt.
- Der Text ist reiner Text (Zeit in UTC, Gerät / Sensor, alt → neu, Meldung). Es gibt **keine automatische Wiederholung** bei Zustellfehlern; der Fehler steht im Log und in der Tabelle.

## Grenzen

- TLS-Zertifikate werden gegen die öffentlichen Zertifizierungsstellen (Mozilla-Liste, `webpki-roots`) geprüft. Ein Mailserver mit privatem oder selbstsigniertem Zertifikat funktioniert
  damit nicht über STARTTLS/TLS; Optionen wären ein eigenes CA-Zertifikat oder „Zertifikat nicht prüfen“ (noch nicht umgesetzt) bzw. ein lokales Relay ohne Verschlüsselung.
- Keine Links zur Weboberfläche in der Mail (der Server kennt seine öffentliche Adresse nicht).
- Keine Eskalation, keine Ruhezeiten, keine Benachrichtigung pro Gerät oder Gruppe.

## Entwicklung und Tests

- `dev/smtp_sink.py [port]` ist ein Wegwerf-SMTP-Server (Standardbibliothek, Port 1025), der alle Mails als `.eml` unter `dev/data/mail/` ablegt.
  In netmon: Server `127.0.0.1`, Port `1025`, Verschlüsselung „Keine“, ohne Anmeldung. Nie ins Netz hängen.
- `src/mail/testsmtp.rs` ist ein kleiner Test-SMTP-Server für die Rust-Tests (optional mit `AUTH PLAIN`, optional mit abgelehnten Empfängern).
- Getestet: Adress- und Einstellungsvalidierung, Versand mit und ohne Anmeldung, Fehlerfälle, verschlüsselte Ablage des Passworts, Auswahl der meldenswerten Wechsel, Zusammenfassen,
  Beenden mit wartender Mail, die Kette „Gerät fällt aus → eine Mail“ und die Weboberfläche (Rechte, CSRF, Passwort nie im HTML, Testmail).
