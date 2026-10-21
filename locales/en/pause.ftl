# Pausing and resuming groups, devices and sensors.
pause = Pause
resume = Resume
pause-heading = Pause “{ $name }”
pause-intro = Paused sensors are not polled and send no notifications. When resumed they start afresh without their old state.
pause-hint-group = All subgroups, devices and sensors of the group are paused with it.
pause-hint-device = All sensors of the device are paused with it.
pause-duration = Duration
pause-reason = Reason (optional)
pause-reason-hint = Shown in the overview and in the log, e.g. “firmware update”.
pause-until-resumed = until resumed
pause-minutes = { $count ->
    [one] 1 minute
   *[other] { $count } minutes
}
pause-hours = { $count ->
    [one] 1 hour
   *[other] { $count } hours
}
pause-days = { $count ->
    [one] 1 day
    [7] 1 week
   *[other] { $count } days
}
pause-until = until
pause-note-own = Paused
pause-note-device = Paused with the device
pause-note-group = Paused with the group “{ $name }”
pause-device-notice = While the device is paused, none of its sensors is polled.
pause-duration-invalid = Please choose one of the offered durations.
pause-reason-invalid = The reason may be at most 200 characters long and must not contain line breaks.
pause-not-paused = This is not (or no longer) paused by itself.
