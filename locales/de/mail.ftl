mail-security-none = Keine Verschlüsselung
mail-security-starttls = STARTTLS
mail-security-tls = TLS (SMTPS)
mail-too-many-recipients = Es sind höchstens { $count } Empfänger erlaubt.
mail-address-invalid = „{ $address }“ ist keine gültige E-Mail-Adresse.
mail-port-invalid = Der Port muss eine Zahl zwischen 1 und 65535 sein.
mail-username-invalid = Der Benutzername ist ungültig.
mail-password-invalid = Das Passwort ist ungültig (zu lang oder Steuerzeichen).
mail-host-missing = Bitte den SMTP-Server angeben.
mail-from-missing = Bitte eine Absenderadresse angeben.
mail-recipients-missing = Bitte mindestens einen Empfänger angeben.
mail-send-failed = Versand fehlgeschlagen: { $detail }
mail-from-invalid = Die Absenderadresse ist nicht gültig.
mail-no-recipients = Es ist kein Empfänger eingetragen.
mail-recipient-invalid = Eine Empfängeradresse ist nicht gültig.
mail-message-invalid = Die Nachricht ist ungültig: { $detail }
mail-server-unusable = Der SMTP-Server ist nicht verwendbar: { $detail }
mail-no-server = Es ist kein SMTP-Server eingetragen.
mail-timeout = Zeitüberschreitung beim Senden.
mail-kind-alert = Alarm
mail-kind-test = Test
mail-security-missing = Bitte die Verschlüsselung wählen.
mail-test-subject = Test-E-Mail
mail-test-body =
    Dies ist eine Test-E-Mail von opennetmon.
    Wenn sie ankommt, sind Server, Zugangsdaten und Empfänger richtig eingestellt.
mail-test-sent = Test-E-Mail an { $recipients } gesendet.
mail-title = E-Mail-Benachrichtigung
mail-subtitle = Bei Ausfall, Warnung und Wiederherstellung kommt eine E-Mail. Wechsel, die kurz nacheinander passieren, werden zusammengefasst. Änderungen gelten sofort.
mail-enabled = Benachrichtigungen aktiv
mail-notify-warnings = Auch bei Warnungen benachrichtigen (sonst nur bei Ausfall und Wiederherstellung)
mail-smtp-server = SMTP-Server
mail-host = Server (Hostname oder IP-Adresse)
mail-security = Verschlüsselung
mail-port-hint = Üblich: STARTTLS 587, TLS (SMTPS) 465, unverschlüsselt 25 (nur für ein vertrauenswürdiges lokales Relay).
mail-login = Anmeldung (optional)
mail-username-placeholder = leer = ohne Anmeldung
mail-not-set = nicht gesetzt
mail-from-to = Absender und Empfänger
mail-from = Absenderadresse
mail-recipients = Empfänger (eine Adresse pro Zeile, höchstens 20)
mail-send-test = Test-E-Mail senden
mail-test-hint = Verwendet die gespeicherten Einstellungen (erst speichern, dann testen) – auch wenn die Benachrichtigung noch aus ist.
mail-log = Letzte Sendeversuche
mail-log-empty = Noch keine E-Mails gesendet.
mail-log-kind = Art
mail-log-subject = Betreff
mail-log-recipients = Empfänger
mail-log-result = Ergebnis
mail-sent = gesendet
mail-failed = Fehler

# Notification mails (in the language chosen on the e-mail page)
mail-language = Sprache der E-Mails
mail-language-hint = Gilt für alle Empfänger und auch für die Test-E-Mail.
alert-subject-down = Ausfall
alert-subject-warning = Warnung
alert-subject-up = Wieder in Ordnung
alert-subject-unknown = Unbekannt
alert-subject-paused = Pausiert
alert-subject-many = { $count } Statuswechsel: { $parts }
alert-count-down = { $count } Ausfall
alert-dependents-paused = Dadurch pausiert: { $sensors ->
        [one] 1 Sensor
       *[other] { $sensors } Sensoren
    } an { $devices ->
        [one] 1 Gerät
       *[other] { $devices } Geräten
    }
alert-count-warning =
    { $count ->
        [one] { $count } Warnung
       *[other] { $count } Warnungen
    }
alert-count-up = { $count } wieder in Ordnung
alert-first-poll = erste Abfrage
mail-language-missing = Bitte die Sprache der E-Mails wählen.
