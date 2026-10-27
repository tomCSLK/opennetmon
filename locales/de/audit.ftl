# Änderungsprotokoll
nav-audit = Änderungen
audit-subtitle = Wer hat wann was geändert, dazu An- und Abmeldungen · nur für Administratoren
audit-empty = Noch keine Einträge.
audit-filter = Einträge filtern …
audit-filtered-user = Benutzer { $user }
audit-when = Zeit
audit-user = Benutzer
audit-action = Aktion
audit-object = Objekt
audit-detail = Änderung

audit-action-created = angelegt
audit-action-changed = geändert
audit-action-deleted = gelöscht
audit-action-signed-in = angemeldet
audit-action-sign-in-failed = Anmeldung fehlgeschlagen
audit-action-signed-out = abgemeldet

audit-kind-group = Gruppe
audit-kind-device = Gerät
audit-kind-sensor = Sensor
audit-kind-profile = SNMP-Profil
audit-kind-user = Benutzer
audit-kind-template = Gerätevorlage
audit-kind-mib = MIB-Datei
audit-kind-mail = E-Mail-Einstellungen
audit-kind-schedule = Zeitplan
audit-kind-session = Anmeldung

audit-change = { $field }: { $old } → { $new }
audit-value = { $field }: { $value }
audit-secret-changed = { $field } geändert
audit-limit = { $channel }, { $limit }: { $old } → { $new }
audit-yes = ja
audit-no = nein
audit-none = keiner
audit-inherited = übernehmen
audit-not-rated = nicht bewertet
audit-unknown-user = unbekannter Benutzername
audit-wrong-password = falsches Passwort
audit-signed-out-idle = nach einer Zeit ohne Aktivität
audit-template-applied =
    { $count ->
        [one] Vorlage „{ $template }“ angewendet, ein Sensor angelegt
       *[other] Vorlage „{ $template }“ angewendet, { $count } Sensoren angelegt
    }
audit-order-saved = Reihenfolge gespeichert
audit-order-reset = Reihenfolge automatisch
audit-order-saved-top = Reihenfolge der obersten Ebene gespeichert
audit-order-reset-top = Reihenfolge der obersten Ebene automatisch
audit-window-added = Fenster hinzugefügt: { $window }
audit-window-deleted = Fenster gelöscht: { $window }

audit-field-name = Name
audit-field-host = Adresse
audit-field-group = Gruppe
audit-field-parent = Übergeordnete Gruppe
audit-field-profile = SNMP-Profil
audit-field-kind = Gerätetyp
audit-field-port = Port
audit-field-interval = Intervall (s)
audit-field-timeout = Timeout (s)
audit-field-retries = Wiederholungen
audit-field-device = Gerät
audit-field-snmp = SNMP
audit-field-username = Benutzername
audit-field-password = Passwort
audit-field-community = Community
audit-field-auth-password = Authentifizierungspasswort
audit-field-priv-password = Verschlüsselungspasswort
audit-field-role = Rolle
audit-field-language = Sprache
audit-field-stay-signed-in = Angemeldet bleiben
audit-field-object-id = sysObjectID
audit-field-modules = Module
audit-field-enabled = Aktiv
audit-field-smtp-host = SMTP-Server
audit-field-security = Verschlüsselung
audit-field-from = Absender
audit-field-recipients = Empfänger
audit-field-notify-warnings = Warnungen melden
audit-field-dependency = Abhängigkeit
audit-field-delay = Verzögerung (s)
audit-field-master = Master-Sensor
audit-field-schedule = Zeitplan
