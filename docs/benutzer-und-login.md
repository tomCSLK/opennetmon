# Benutzer und Anmeldung

## Rollen

| Rolle | Rechte |
|---|---|
| Administrator (`admin`) | alles, inklusive Verwaltung (folgt mit den nächsten MVP-Schritten) |
| Leser (`viewer`) | nur ansehen |

Handler, die nur Administratoren erlauben sollen, rufen `CurrentUser::require_admin()` auf.

## Benutzer anlegen (Kommandozeile)

Der erste Administrator wird auf dem Server angelegt, es gibt keine Standardzugangsdaten:

```bash
netmon --config /etc/netmon/config.toml user add admin --role admin
netmon --config /etc/netmon/config.toml user add lena --role viewer
netmon --config /etc/netmon/config.toml user passwd lena    # beendet alle Sitzungen des Benutzers
netmon --config /etc/netmon/config.toml user list
```

Das Passwort wird abgefragt (zweimal, ohne Anzeige). Bei Eingabe über eine Pipe wird eine Zeile von stdin gelesen.
Regeln: Benutzername 3 bis 32 Zeichen (`a-z A-Z 0-9 . _ -`, Groß-/Kleinschreibung egal), Passwort 10 bis 128 Zeichen.

## Sicherheitsmerkmale

- Passwörter: Argon2id mit zufälligem Salt. Auch bei unbekanntem Benutzer wird ein Hash geprüft, damit Antwortzeit und Meldung gleich bleiben.
- Sitzung: zufälliges 256-Bit-Token im Cookie `netmon_session` (`HttpOnly`, `SameSite=Strict`, `Secure` laut Config). In der Datenbank steht nur der SHA-256-Wert des Tokens.
- Lebensdauer: absolut `session_ttl_hours` (Standard 168 Stunden). Abgelaufene Sitzungen werden beim Anmelden gelöscht.
- CSRF: Anmeldeformular per Double-Submit-Cookie (`netmon_login`), alle weiteren POST-Formulare mit einem Token pro Sitzung.
- Antwort-Header: `Content-Security-Policy` (nur eigene Ressourcen), `X-Frame-Options: DENY`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: same-origin`.
- Passwörter erscheinen nie in Logs. Fehlgeschlagene Anmeldungen werden ohne Benutzernamen protokolliert.

## Konfiguration

| Schlüssel | Standard | Bedeutung |
|---|---|---|
| `cookie_secure` | `true` | Cookies nur über HTTPS senden. Nur für Entwicklung über reines HTTP auf `false` setzen |
| `session_ttl_hours` | `168` | Sitzungsdauer in Stunden (1 bis 2160) |

## Bekannte Lücke

Es gibt noch keine Begrenzung der Anmeldeversuche. Hinter dem Reverse Proxy kann Caddy oder nginx
(`limit_req`) das übernehmen; eine eigene Begrenzung folgt später.
