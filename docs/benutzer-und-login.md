# Benutzer und Anmeldung

## Rollen

| Rolle | Rechte |
|---|---|
| Administrator (`admin`) | alles, inklusive Benutzerverwaltung |
| Leser (`viewer`) | nur ansehen |

Handler, die nur Administratoren erlauben sollen, rufen `CurrentUser::require_admin()` auf.

## Benutzer verwalten (Oberfläche, `/users`, nur Administratoren)

Unter **Einstellungen → Benutzer**:

- **Liste** mit Rolle, Anlagezeitpunkt und Zahl der offenen Sitzungen; das eigene Konto ist mit „(du)“ markiert.
- **Neuer Benutzer:** Benutzername, Rolle (Leser oder Administrator), Passwort zweimal. Bei Fehlern bleiben Name und Rolle stehen, Passwörter nie.
- **Je Benutzer** (`/users/<id>`):
  - **Rolle ändern** – gilt sofort, auch in offenen Sitzungen. Der **letzte Administrator** kann nicht zum Leser werden (in derselben
    SQL-Anweisung geprüft, damit zwei gleichzeitige Änderungen nicht beide Administratoren entfernen). Wer sich selbst zum Leser macht, landet auf der Übersicht.
  - **Passwort setzen** – beendet alle Sitzungen des Benutzers; beim eigenen Konto bleibt die aktuelle Sitzung angemeldet.
  - **Löschen** – mit Bestätigungshaken, meldet alle Sitzungen ab; nie das eigene Konto und nie den letzten Administrator.
- Aktionen werden protokolliert (wer, was, welcher Benutzer), Passwörter nie.

## Mein Konto (`/account`, alle Benutzer)

Erreichbar über Name und Rolle unten in der Seitenleiste. Zeigt Benutzername und Rolle und ändert das **eigene Passwort**:

- Nötig sind das bisherige Passwort und das neue zweimal (10 bis 128 Zeichen, anders als das bisherige).
- Falsche bisherige Passwörter zählen wie fehlgeschlagene Anmeldungen für den Benutzernamen (`auth::throttle`); ab 5 Fehlversuchen wird gebremst.
- Danach werden alle anderen Sitzungen des Benutzers abgemeldet, die aktuelle bleibt angemeldet.

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
| `client_ip_header` | nicht gesetzt | Hinter einem Reverse Proxy: `"x-forwarded-for"` (letzter Eintrag) oder `"x-real-ip"`, damit die Begrenzung die echte Client-Adresse sieht (siehe unten) |

## Begrenzung der Anmeldeversuche (`src/auth/throttle.rs`)

Fehlgeschlagene Anmeldungen werden in einem gleitenden Fenster von 15 Minuten gezählt, **je Benutzername** und **je Client-Adresse**. Ab einer Schwelle werden weitere
Versuche **abgelehnt** (HTTP 429 mit `Retry-After`, ohne das Passwort zu prüfen – auch nicht das richtige), und die Wartezeit verdoppelt sich mit jedem weiteren Fehlversuch:

| | Schwelle | Wartezeit | Obergrenze |
|---|---|---|---|
| je Benutzername (Groß-/Kleinschreibung egal) | 5 Fehlversuche | 5 s, 10 s, 20 s … | 60 s |
| je Adresse (IPv6: je /64) | 10 Fehlversuche | 30 s, 60 s, 120 s … | 15 min |

Der Benutzername wird bewusst nur **gebremst, nicht gesperrt**: Ein Angreifer kann den echten Administrator so höchstens eine Minute aufhalten, selbst wenn er die Adresse wechselt,
und kommt dabei auf etwa ein Passwort pro Minute. Die strengere Adressgrenze verhindert, dass ein einzelner Rechner viele Konten durchprobiert.
Ein erfolgreicher Login setzt die Zähler von Benutzername und Adresse zurück; ein Neustart des Servers ebenfalls (der Zustand liegt nur im Speicher und ist begrenzt).
Die abgelehnte Antwort ist für richtige und falsche Benutzernamen gleich, so dass sich Konten darüber nicht erraten lassen.

**Hinter einem Reverse Proxy** sieht der Server nur die Adresse des Proxys. Mit `client_ip_header = "x-forwarded-for"` (Caddy und nginx hängen die Client-Adresse an;
verwendet wird der *letzte* Eintrag, also der vom eigenen Proxy) bzw. `"x-real-ip"` wird stattdessen die echte Adresse genommen. Nur setzen, wenn der Dienst ausschließlich über den Proxy
erreichbar ist (Standard: `127.0.0.1`), sonst könnte ein Client die Adresse fälschen und die Adressgrenze umgehen (die Grenze je Benutzername bleibt wirksam).
Ohne diese Einstellung zählen alle Zugriffe über den Proxy als eine Adresse – dann greift die strenge Adressgrenze für alle zusammen.
