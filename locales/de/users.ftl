# Anmeldung, „Mein Konto“ und Benutzerverwaltung.

## Anmeldung

login-title = Anmelden
login-tagline = Netzwerk-Monitoring
login-submit = Anmelden
login-free-software = opennetmon ist freie Software unter der AGPL-3.0
login-failed = Benutzername oder Passwort ist falsch.
login-busy = Gerade laufen zu viele Anmeldungen gleichzeitig. Bitte in ein paar Sekunden erneut versuchen.
login-throttled = Zu viele fehlgeschlagene Anmeldeversuche. Bitte in { $wait } erneut versuchen.
login-signed-out-idle =
    { $minutes ->
        [one] Du wurdest nach { $minutes } Minute ohne Aktivität abgemeldet.
       *[other] Du wurdest nach { $minutes } Minuten ohne Aktivität abgemeldet.
    }
login-wait-seconds =
    { $seconds ->
        [one] { $seconds } Sekunde
       *[other] { $seconds } Sekunden
    }
login-wait-minutes =
    { $minutes ->
        [one] { $minutes } Minute
       *[other] { $minutes } Minuten
    }

## Prüfung von Benutzername und Passwort

username-length = Der Benutzername muss 3 bis 32 Zeichen lang sein.
username-chars = Erlaubt sind Buchstaben, Ziffern sowie . _ -
username-taken = Der Benutzername ist bereits vergeben.
password-too-short = Das Passwort muss mindestens 10 Zeichen lang sein.
password-too-long = Das Passwort darf höchstens 128 Zeichen lang sein.
password-mismatch = Die beiden Passwörter stimmen nicht überein.

## Mein Konto

account-language-hint = „Wie im Browser“ folgt der Spracheinstellung des Browsers; ohne passende Sprache gilt Deutsch.
account-password-title = Passwort ändern
account-current-password = Bisheriges Passwort
account-password-hint = 10 bis 128 Zeichen. Deine anderen Sitzungen (z. B. auf dem Handy) werden abgemeldet, diese bleibt angemeldet.
account-too-many-attempts = Zu viele falsche Passwörter. Bitte später erneut versuchen.
account-current-wrong = Das bisherige Passwort ist falsch.
account-password-unchanged = Das neue Passwort muss sich vom bisherigen unterscheiden.

## Benutzerverwaltung

users-subtitle = Administratoren dürfen alles ändern, Leser nur ansehen.
users-created = Angelegt
users-sessions = Sitzungen
users-me = (du)
users-stays-signed-in = bleibt angemeldet
users-new = Neuer Benutzer
users-new-hint = Benutzername 3 bis 32 Zeichen (Buchstaben, Ziffern, . _ -), Passwort 10 bis 128 Zeichen. Das Passwort gibst du der Person auf einem sicheren Weg weiter.
users-create = Benutzer anlegen
user-role-missing = Bitte eine Rolle wählen.
user-role-hint = Gilt sofort, auch in offenen Sitzungen. Der letzte Administrator bleibt Administrator.
user-last-admin = Das ist der letzte Administrator. Lege zuerst einen weiteren Administrator an.
user-password-title = Passwort setzen
user-password-hint-me = Deine anderen Sitzungen werden beendet, diese bleibt angemeldet.
user-password-hint-other = Alle Sitzungen von { $name } werden beendet.
user-delete = Benutzer löschen
user-delete-confirm = { $name } wirklich löschen (meldet alle Sitzungen ab)
user-delete-self = Das eigene Konto lässt sich nicht löschen; das muss ein anderer Administrator tun.
user-idle-title = Automatisch abmelden
user-stay-signed-in = Ausnahme: angemeldet bleiben (z. B. für einen Wandbildschirm)
user-idle-hint =
    { $minutes ->
        [one] Ohne Haken wird der Benutzer nach { $minutes } Minute ohne Klick oder Eingabe abgemeldet
       *[other] Ohne Haken wird der Benutzer nach { $minutes } Minuten ohne Klick oder Eingabe abgemeldet
    }; das automatische Aktualisieren zählt nicht als Aktivität. Mit Haken bleibt er angemeldet, bis die Sitzung nach { $hours } Stunden abläuft. Nur für Anzeigen gedacht, an denen niemand arbeitet; gilt sofort, auch in offenen Sitzungen.
user-idle-off = Das automatische Abmelden ist in der Konfiguration abgeschaltet (session_idle_minutes = 0). Die Ausnahme wirkt erst, wenn es eingeschaltet ist.
