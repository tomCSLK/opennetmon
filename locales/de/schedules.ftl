# Wartungsfenster (Zeitpläne)
nav-schedules = Wartungsfenster
schedules-subtitle = Zeitpläne für geplante Unterbrechungen: in den Fenstern pausiert, ohne Benachrichtigung
schedule = Zeitplan
schedule-windows = Fenster
schedule-window = Fenster
schedule-used = Verwendet
schedule-used-count = { $count ->
        [one] einem Objekt zugeordnet
        [0] keinem Objekt zugeordnet
       *[other] { $count } Objekten zugeordnet
    }
schedule-zone = Zeitzone des Servers: { $zone }
schedules-none = Noch keine Zeitpläne.
schedules-none-hint = Legen Sie unten einen an, z. B. „Nachts“ für das tägliche Backup.
schedule-new = Neuer Zeitplan
schedule-name-placeholder = z. B. Nachts
schedule-create = Anlegen
schedules-how = So funktioniert es
schedules-how-windows = Ein Zeitplan hat Fenster: wöchentlich (Wochentage, Beginn, Dauer) oder einmalig (Beginn und Ende). Die Zeiten gelten in der Zeitzone des Servers.
schedules-how-assign = Zugeordnet wird er auf der Seite einer Gruppe, eines Geräts oder Sensors unter „⋯ Aktionen“ → „Wartungsfenster“; Untergruppen, Geräte und Sensoren übernehmen ihn.
schedules-how-effect = Während eines Fensters sind die Objekte pausiert: keine Abfragen, keine Mails. Abhängige Objekte folgen. Danach geht es ohne nachträgliche Mail weiter.
schedule-no-windows = Noch keine Fenster
schedule-no-windows-hint = Fügen Sie unten ein wöchentliches oder einmaliges Fenster hinzu.
schedule-every-day = täglich
schedule-workdays = Mo–Fr
schedule-weekend = Sa und So
weekday-1 = Mo
weekday-2 = Di
weekday-3 = Mi
weekday-4 = Do
weekday-5 = Fr
weekday-6 = Sa
weekday-7 = So
schedule-weekly = { $days } ab { $time } Uhr, { $duration }
schedule-once = { $start } bis { $end }
schedule-open-until = offen bis { $time }
schedule-next = nächster Beginn { $time }
schedule-no-next = kein weiteres Fenster
schedule-window-over = vorbei
schedule-add-weekly = Wöchentliches Fenster hinzufügen
schedule-add-once = Einmaliges Fenster hinzufügen
schedule-weekdays = Wochentage
schedule-start-time = Beginn (Uhrzeit)
schedule-duration = Dauer in Minuten
schedule-weekly-hint = Ein Fenster über Mitternacht gehört zu dem Tag, an dem es beginnt. Höchstens eine Woche.
schedule-start = Beginn
schedule-end = Ende
schedule-add = Hinzufügen
schedule-delete = Zeitplan löschen
schedule-delete-hint = Zugeordnete Gruppen, Geräte und Sensoren übernehmen danach wieder den Zeitplan darüber.
schedule-name-required = Bitte einen Namen angeben.
schedule-name-invalid = Der Name darf höchstens { $max } Zeichen lang sein und keine Steuerzeichen enthalten.
schedule-name-taken = Ein Zeitplan mit diesem Namen existiert bereits.
schedule-time-invalid = Bitte eine Uhrzeit wie 03:00 angeben.
schedule-duration-invalid = Die Dauer muss zwischen 1 und { $max } Minuten liegen.
schedule-weekdays-required = Bitte mindestens einen Wochentag wählen.
schedule-datetime-invalid = Bitte Datum und Uhrzeit angeben.
schedule-end-before-start = Das Ende muss nach dem Beginn liegen.
schedule-required = Bitte einen Zeitplan wählen.
schedule-kind-invalid = Unbekannte Art von Fenster.
flash-schedule-created = Zeitplan angelegt. Fügen Sie jetzt Fenster hinzu.
flash-schedule-saved = Zeitplan gespeichert.
flash-schedule-deleted = Zeitplan gelöscht.
flash-window-added = Fenster hinzugefügt.
flash-window-deleted = Fenster gelöscht.
flash-maintenance-saved = Wartungsfenster gespeichert.

# Zuordnung
maintenance = Wartungsfenster
maintenance-heading = Wartungsfenster für „{ $name }“
maintenance-applies = Gilt jetzt: { $value }
maintenance-intro = Während der Fenster des Zeitplans ist das Objekt pausiert und sendet keine Benachrichtigungen.
maintenance-parent = Vom übergeordneten Objekt übernehmen
maintenance-schedule = Diesen Zeitplan verwenden
maintenance-choose = Zeitplan wählen …
maintenance-no-schedules = Noch keine Zeitpläne angelegt:
maintenance-none = Kein Wartungsfenster
maintenance-none-hint = Gilt auch, wenn darüber ein Zeitplan zugeordnet ist.
maintenance-none-applies = keins
maintenance-schedule-named = „{ $name }“
pause-note-schedule = Wartungsfenster „{ $name }“
log-schedule-started = Wartungsfenster
log-schedule-started-message = „{ $name }“: { $sensors ->
        [one] 1 Sensor
       *[other] { $sensors } Sensoren
    } an { $devices ->
        [one] 1 Gerät
       *[other] { $devices } Geräten
    } pausiert
log-schedule-ended = Wartungsfenster beendet
log-schedule-ended-message = „{ $name }“: Sensoren laufen weiter
log-scope-schedule = Zeitplan
