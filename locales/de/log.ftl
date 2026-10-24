# Protokoll: Warnungen, Ausfälle und ihr Ende über alle Sensoren, dazu Pausen.
log = Protokoll
log-count = { $count ->
    [one] 1 Eintrag
   *[other] { $count } Einträge
}
log-kept = gespeichert werden die letzten { $count } Statuswechsel und Pausen
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
