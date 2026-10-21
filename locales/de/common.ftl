# Allgemeine Texte: Navigation, Rollen, häufige Wörter, Fehlerseiten, Sprache, Meldungen nach Aktionen.
# Deutsch ist die Ausgangssprache; jede andere Sprache braucht dieselben Nachrichten mit denselben Variablen.

## Navigation und Seitenrahmen

nav-menu = Menü
nav-main = Hauptnavigation
nav-monitoring = Monitoring
nav-overview = Übersicht
nav-settings = Einstellungen
nav-credentials = SNMP-Profile
nav-users = Benutzer
nav-mail = E-Mail
nav-templates = Gerätevorlagen
nav-mibs = MIB-Dateien
nav-account = Mein Konto
nav-theme = Design
nav-theme-toggle = Design umschalten
nav-logout = Abmelden
source-code = Quellcode

## Rollen

role = Rolle
role-admin = Administrator
role-viewer = Leser
role-admin-option = Administrator – alles ändern
role-viewer-option = Leser – nur ansehen

## Häufige Wörter

save = Speichern
edit = Bearbeiten
delete = Löschen
username = Benutzername
password = Passwort
password-repeat = Passwort wiederholen
password-repeat-short = Wiederholen
password-new = Neues Passwort
password-new-repeat = Neues Passwort wiederholen
confirm-delete = Bitte das Löschen bestätigen.
refresh = Aktualisieren
refresh-auto = Automatisch
refresh-auto-title = Seite alle 60 Sekunden neu laden (wartet, solange du etwas eingibst)

## Fehlerseiten

error-forbidden = Zugriff verweigert
error-not-found = Nicht gefunden
error-internal = Interner Fehler

## Sprache

language = Sprache
language-browser = Wie im Browser
language-invalid = Diese Sprache gibt es nicht.

## Meldungen nach Aktionen

flash-group-created = Gruppe angelegt.
flash-group-updated = Gruppe gespeichert.
flash-group-deleted = Gruppe gelöscht.
flash-device-created = Gerät angelegt.
flash-device-created-ping = Gerät ohne SNMP angelegt; der Ping-Sensor ist aktiv.
flash-device-updated = Gerät gespeichert.
flash-device-deleted = Gerät gelöscht.
flash-sensor-created = Sensor angelegt.
flash-sensors-added = Sensoren angelegt.
flash-sensor-deleted = Sensor gelöscht.
flash-sensor-paused = Sensor deaktiviert.
flash-sensor-resumed = Sensor aktiviert.
flash-values-added-inactive = Sensoren angelegt – sie sind noch inaktiv, unten unter „Inaktive Sensoren“ aktivieren.
flash-values-added-active = Sensoren angelegt und aktiviert.
flash-sensors-activated = Sensoren aktiviert.
flash-sensors-deactivated = Sensoren deaktiviert.
flash-sensors-deleted = Sensoren gelöscht.
flash-thresholds-saved = Schwellwerte gespeichert.
flash-sensor-renamed = Sensor umbenannt.
flash-states-saved = Bewertung der Statuswerte gespeichert. Sie gilt ab der nächsten Abfrage.
flash-states-reset = Bewertung der Statuswerte entfernt.
flash-order-saved = Reihenfolge gespeichert.
flash-order-reset = Reihenfolge zurückgesetzt: wieder automatisch sortiert.
flash-template-created = Vorlage gespeichert. Sie lässt sich beim Anlegen eines Geräts oder mit „Vorlage anwenden“ nutzen.
flash-template-saved = Vorlage gespeichert.
flash-template-deleted = Vorlage gelöscht.
flash-user-created = Benutzer angelegt.
flash-user-role-saved = Rolle gespeichert.
flash-user-language-saved = Sprache gespeichert.
flash-user-password-saved = Passwort gesetzt.
flash-user-deleted = Benutzer gelöscht.
flash-account-password-saved = Passwort geändert. Deine anderen Sitzungen wurden abgemeldet.
flash-language-saved = Sprache gespeichert.
flash-mail-saved = E-Mail-Einstellungen gespeichert.
flash-mib-deleted = MIB-Datei gelöscht.
flash-profile-created = Profil angelegt.
flash-profile-updated = Profil gespeichert.
flash-profile-deleted = Profil gelöscht.

status-up = In Ordnung
status-warning = Warnung
status-down = Ausfall
status-unknown = Unbekannt
status-paused = Inaktiv
no-sensors = keine Sensoren
sensors = Sensoren
name = Name
cancel = Abbrechen
order = Reihenfolge
edit-named = { $name } bearbeiten
every-seconds = alle { $seconds } s
count-devices =
    { $count ->
        [one] { $count } Gerät
       *[other] { $count } Geräte
    }
count-sensors =
    { $count ->
        [one] { $count } Sensor
       *[other] { $count } Sensoren
    }
count-inactive = { $count } inaktiv
count-warning = { $count } Warnung
count-down = { $count } Ausfall
duration-minutes = { $minutes } Min.
duration-hours-minutes = { $hours } Std. { $minutes } Min.
duration-days-hours-minutes =
    { $days ->
        [one] { $days } Tag
       *[other] { $days } Tage
    } { $hours } Std. { $minutes } Min.
event-first-poll = erste Abfrage: { $status }
snmp-timeout = Keine Antwort innerhalb des Timeouts. Mögliche Ursachen: Gerät oder Port nicht erreichbar, SNMP nicht aktiviert, falsche Community oder falsche SNMPv3-Benutzerdaten.
snmp-auth = Anmeldung fehlgeschlagen. Benutzer, Passwort, Authentifizierungs- oder Verschlüsselungsverfahren stimmen nicht mit dem Gerät überein.
snmp-unreachable = Das Gerät ist nicht erreichbar oder der SNMP-Port ist geschlossen.
snmp-request = Die Anfrage konnte nicht aufgebaut werden: { $detail }
snmp-protocol = Unerwartete Antwort des Geräts: { $detail }

profile-summary-user = Benutzer { $user }

duration-minutes-seconds = { $minutes } Min. { $seconds } s
channel-in = Eingehend
channel-out = Ausgehend
channel-rtt = Antwortzeit
channel-uptime = Laufzeit
channel-oper-status = Status
channel-value = Wert
threshold-not-a-number = Ein Schwellwert muss eine Zahl sein.
threshold-not-finite = Ein Schwellwert muss eine endliche Zahl sein.

unknown-action = Unbekannte Aktion.
inactive = inaktiv

status = Status
sensor = Sensor
message = Meldung
actions = Aktionen

active = aktiv

backup-path-not-utf8 = Der Pfad der Sicherung ist kein gültiges UTF-8.
backup-exists = { $path } existiert bereits; eine vorhandene Sicherung wird nie überschrieben.
