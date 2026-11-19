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
alert-count-down =
    { $count ->
        [one] { $count } Ausfall
       *[other] { $count } Ausfälle
    }
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
mail-language-missing = Bitte die Sprache der E-Mails wählen.
mail-ui-url = Adresse der Oberfläche (optional)
mail-ui-url-hint = Für Knöpfe und Links in den E-Mails, z. B. https://opennetmon.example.org. Leer lassen, dann enthalten die E-Mails keine Links.
mail-ui-url-invalid = Die Adresse der Oberfläche muss mit http:// oder https:// beginnen und darf keine Anmeldedaten, keine Abfrage (?) und keinen Anker (#) enthalten.

# Content of the notification mails
mail-date = { $day }.{ $month }.{ $year }
mail-footer-sent =
    Gesendet von opennetmon { $version } an { $count ->
        [one] 1 Empfänger
       *[other] { $count } Empfänger
    }
mail-footer-zone = Zeiten in { $zone }
mail-footer-batch = Wechsel innerhalb von { $seconds } s kommen in einer E-Mail
mail-footer-settings = Benachrichtigungen einstellen
mail-open-sensor = Sensor öffnen
mail-open-device = Gerät öffnen
mail-open-log = Protokoll
mail-open-report = Bericht 7 Tage
mail-open-thresholds = Grenzwerte ändern
mail-open-overview = Übersicht öffnen
mail-open-settings = Einstellungen öffnen
mail-since-time = seit { $time }
mail-fact-since = Seit
mail-fact-last-value = Letzter Wert
mail-fact-value = Aktueller Wert
mail-fact-at = um { $time }
mail-fact-limits = Grenzwerte
mail-fact-availability = Verfügbarkeit 7 Tage
mail-fact-outages = Ausfälle 7 Tage
mail-fact-down-lasted = Ausfall dauerte
mail-fact-warning-lasted = Warnung dauerte
mail-fact-device = Gerät
mail-outages-none = kein Ausfall
mail-outages =
    { $count ->
        [one] 1 Ausfall
       *[other] { $count } Ausfälle
    }, zusammen { $duration }
mail-outages-total = zusammen { $duration }
mail-limit-warn-above = Warnung über { $value }
mail-limit-warn-below = Warnung unter { $value }
mail-limit-down-above = Ausfall über { $value }
mail-limit-down-below = Ausfall unter { $value }
mail-chart-title = { $channel }, letzte 6 Stunden
mail-chart-max = Max. { $value }
mail-impact = Auswirkung
mail-impact-paused = Dadurch pausiert (hängen von diesem Sensor ab), keine eigenen E-Mails:
mail-impact-sensors =
    { $count ->
        [one] 1 Sensor
       *[other] { $count } Sensoren
    }
mail-impact-more =
    { $count ->
        [one] und 1 weiteres Gerät
       *[other] und { $count } weitere Geräte
    }
mail-impact-resumed =
    { $sensors ->
        [one] Der pausierte Sensor
       *[other] Die { $sensors } pausierten Sensoren
    } auf { $devices ->
        [one] 1 Gerät
       *[other] { $devices } Geräten
    } ({ $names }) { $sensors ->
        [one] läuft
       *[other] laufen
    } wieder; { $sensors ->
        [one] er wird
       *[other] sie werden
    } ab jetzt normal abgefragt.
mail-subject-down-lasted = Ausfall { $duration }
mail-subject-warning-lasted = Warnung { $duration }
mail-pre-up = Wieder in Ordnung seit { $time }.
mail-pre-down-lasted = Der Ausfall dauerte { $duration }.
mail-pre-warning-lasted = Die Warnung dauerte { $duration }.
mail-batch-title = { $count } Statuswechsel
mail-batch-note = zusammengefasst, weil sie kurz nacheinander kamen
mail-counter-down =
    { $count ->
        [one] Ausfall
       *[other] Ausfälle
    }
mail-counter-warning =
    { $count ->
        [one] Warnung
       *[other] Warnungen
    }
mail-counter-up = wieder in Ordnung
mail-cause = Vermutliche Ursache
mail-cause-silent = Sie melden sich nicht einzeln.
mail-all-changes = Alle Wechsel
mail-more-changes = … und { $count } weitere
mail-test-title = Die E-Mail-Einstellungen funktionieren
mail-test-ok = Test erfolgreich
mail-test-by = Ausgelöst von { $user } unter Einstellungen → E-Mail
mail-test-pre = Die E-Mail-Einstellungen funktionieren. So sehen Benachrichtigungen aus.
mail-test-server = SMTP-Server
mail-test-login-yes = mit Anmeldung
mail-test-login-no = ohne Anmeldung
mail-test-recipients = Empfänger
mail-test-notify = Benachrichtigt bei
mail-test-notify-all = Ausfall, Warnung, Wiederherstellung
mail-test-notify-down = Ausfall, Wiederherstellung
mail-test-language = Sprache
mail-test-look = So sehen Benachrichtigungen aus
mail-test-look-text = Mit Kennzahlen und dem Verlauf der letzten 6 Stunden, mit eingetragener Adresse der Oberfläche auch mit Knöpfen dorthin. Wechsel, die kurz nacheinander kommen, fasst eine Sammelmail zusammen.
mail-duration-days =
    { $days ->
        [one] 1 Tag
       *[other] { $days } Tage
    }
mail-duration-days-hours =
    { $days ->
        [one] 1 Tag
       *[other] { $days } Tage
    } { $hours } Std.
mail-date-short = { $day }.{ $month }.
mail-before-up = vorher { $duration } in Ordnung
mail-before-warning = vorher { $duration } mit Warnung
mail-before-down = vorher { $duration } ausgefallen
mail-before-unknown = vorher { $duration } unbekannt
mail-before-paused = vorher { $duration } pausiert
mail-fact-now = Aktuell
mail-fact-average = Durchschnitt 6 h: { $value }
mail-since-beyond = seit { $time } { $limit }
mail-beyond-above = über { $value }
mail-beyond-below = unter { $value }
mail-fact-trend = Tendenz
mail-trend = { $delta } in 2 Std.
mail-trend-before = vorher { $range }
mail-fact-peak = Höchstwert 7 Tage
mail-fact-low = Tiefstwert 7 Tage
mail-peak-at = am { $date } um { $time }
mail-up-answers = Antwortet wieder – { $value }
mail-up-value = Wieder in Ordnung – { $value }
mail-other-sensors = Weitere Kanäle dieses Geräts
mail-chart-warn-above = Warnung { $value }
mail-chart-warn-below = Warnung unter { $value }
mail-chart-down-above = Ausfall { $value }
mail-chart-down-below = Ausfall unter { $value }
mail-chart-no-answer = keine Antwort
mail-chart-outage = Ausfall
mail-chart-alt = { $channel } der letzten 6 Stunden
mail-chart-alt-down = { $channel } der letzten 6 Stunden, seit { $time } { $state }
mail-chart-alt-up = { $channel } der letzten 6 Stunden mit dem Ausfall von { $from } bis { $to }
mail-chart-alt-warning = { $channel } der letzten 6 Stunden, seit { $time } { $limit }
