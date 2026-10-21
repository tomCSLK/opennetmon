# Log: warnings, outages and their end across all sensors.
log = Log
log-count = { $count ->
    [one] 1 entry
   *[other] { $count } entries
}
log-kept = the last { $count } status changes are kept
log-empty = No warnings or outages yet.
log-duration = Duration
log-before = before: { $status }
log-after = after { $duration }
log-ongoing = ongoing
