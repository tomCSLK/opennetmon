# Log: warnings, outages and their end across all sensors, plus pauses.
log = Log
log-count = { $count ->
    [one] 1 entry
   *[other] { $count } entries
}
log-kept = the last { $count } status changes, pauses and dependencies are kept
log-empty = No warnings, outages or pauses yet.
log-paused-by = by { $user }
log-resumed = Resumed
log-resumed-by = by { $user }
log-expired = Pause ended
log-expired-message = resumed automatically
log-scope-device = whole device
log-scope-group = whole group
log-duration = Duration
log-before = before: { $status }
log-after = after { $duration }
log-ongoing = ongoing
log-filter = Search the log
log-none-match = No entry matches the search.
log-scope-device-title = Device { $device }
log-scope-sensor = Sensor { $sensor } on { $device }
log-show-all = Whole log
log-dependency-active = Dependency active
log-dependency-active-message = { $sensors ->
        [one] 1 sensor
       *[other] { $sensors } sensors
    } on { $devices ->
        [one] 1 device
       *[other] { $devices } devices
    } paused
log-dependency-ended = Dependency ended
log-dependency-ended-message = dependent sensors run again
