# Protokoll: Warnungen, Ausfälle und ihr Ende über alle Sensoren.
log = Protokoll
log-count = { $count ->
    [one] 1 Eintrag
   *[other] { $count } Einträge
}
log-kept = gespeichert werden die letzten { $count } Statuswechsel
log-empty = Noch keine Warnungen oder Ausfälle.
log-duration = Dauer
log-before = vorher { $status }
log-after = nach { $duration }
log-ongoing = dauert an
