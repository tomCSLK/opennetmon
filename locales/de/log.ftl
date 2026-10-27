# Protokoll: Warnungen, Ausfälle und ihr Ende über alle Sensoren, dazu Pausen.
log = Protokoll
log-count = { $count ->
    [one] 1 Eintrag
   *[other] { $count } Einträge
}
log-kept = gespeichert werden die letzten { $count } Statuswechsel, Pausen und Abhängigkeiten
log-empty = Noch keine Warnungen, Ausfälle oder Pausen.
log-paused-by = von { $user }
log-resumed = Fortgesetzt
log-resumed-by = von { $user }
log-expired = Pause abgelaufen
log-expired-message = automatisch fortgesetzt
log-scope-device = ganzes Gerät
log-scope-group = ganze Gruppe
log-duration = Dauer
log-before = vorher { $status }
log-after = nach { $duration }
log-ongoing = dauert an
log-filter = Protokoll durchsuchen
log-none-match = Kein Eintrag passt zur Suche.
log-scope-device-title = Gerät { $device }
log-scope-sensor = Sensor { $sensor } auf { $device }
log-show-all = Ganzes Protokoll
log-dependency-active = Abhängigkeit aktiv
log-dependency-active-message = { $sensors ->
        [one] 1 Sensor
       *[other] { $sensors } Sensoren
    } an { $devices ->
        [one] 1 Gerät
       *[other] { $devices } Geräten
    } pausiert
log-dependency-ended = Abhängigkeit beendet
log-dependency-ended-message = abhängige Sensoren laufen weiter
