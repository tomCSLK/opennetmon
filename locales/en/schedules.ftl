# Maintenance windows (schedules)
nav-schedules = Maintenance windows
schedules-subtitle = Schedules for planned interruptions: paused during their windows, without notifications
schedule = Schedule
schedule-windows = Windows
schedule-window = Window
schedule-used = Used
schedule-used-count = { $count ->
        [one] assigned to one object
        [0] not assigned
       *[other] assigned to { $count } objects
    }
schedule-zone = Time zone of the server: { $zone }
schedules-none = No schedules yet.
schedules-none-hint = Add one below, e.g. “Nightly” for the daily backup.
schedule-new = New schedule
schedule-name-placeholder = e.g. Nightly
schedule-create = Add
schedules-how = How it works
schedules-how-windows = A schedule has windows: weekly (weekdays, start, duration) or once (start and end). The times are in the time zone of the server.
schedules-how-assign = Assign it on the page of a group, device or sensor under “⋯ Actions” → “Maintenance window”; subgroups, devices and sensors take it over.
schedules-how-effect = During a window the objects are paused: no polls, no mails. Dependent objects follow. Afterwards monitoring continues without a belated mail.
schedule-no-windows = No windows yet
schedule-no-windows-hint = Add a weekly or a single window below.
schedule-every-day = daily
schedule-workdays = Mon–Fri
schedule-weekend = Sat and Sun
weekday-1 = Mon
weekday-2 = Tue
weekday-3 = Wed
weekday-4 = Thu
weekday-5 = Fri
weekday-6 = Sat
weekday-7 = Sun
schedule-weekly = { $days } from { $time }, { $duration }
schedule-once = { $start } to { $end }
schedule-open-until = open until { $time }
schedule-next = next start { $time }
schedule-no-next = no further window
schedule-window-over = over
schedule-add-weekly = Add a weekly window
schedule-add-once = Add a single window
schedule-weekdays = Weekdays
schedule-start-time = Start (time of day)
schedule-duration = Duration in minutes
schedule-weekly-hint = A window over midnight belongs to the day it starts on. At most one week.
schedule-start = Start
schedule-end = End
schedule-add = Add
schedule-delete = Delete schedule
schedule-delete-hint = Assigned groups, devices and sensors then take over the schedule above them again.
schedule-name-required = Please enter a name.
schedule-name-invalid = The name may be at most { $max } characters long and must not contain control characters.
schedule-name-taken = A schedule with this name already exists.
schedule-time-invalid = Please enter a time like 03:00.
schedule-duration-invalid = The duration must be between 1 and { $max } minutes.
schedule-weekdays-required = Please choose at least one weekday.
schedule-datetime-invalid = Please enter a date and time.
schedule-end-before-start = The end must be after the start.
schedule-required = Please choose a schedule.
schedule-kind-invalid = Unknown kind of window.
flash-schedule-created = Schedule added. Now add its windows.
flash-schedule-saved = Schedule saved.
flash-schedule-deleted = Schedule deleted.
flash-window-added = Window added.
flash-window-deleted = Window deleted.
flash-maintenance-saved = Maintenance window saved.

# Assignment
maintenance = Maintenance window
maintenance-heading = Maintenance window for “{ $name }”
maintenance-applies = Applies now: { $value }
maintenance-intro = During the windows of the schedule the object is paused and sends no notifications.
maintenance-parent = Take it from the object above
maintenance-schedule = Use this schedule
maintenance-choose = Choose a schedule …
maintenance-no-schedules = No schedules yet:
maintenance-none = No maintenance window
maintenance-none-hint = Also when a schedule is assigned above.
maintenance-none-applies = none
maintenance-schedule-named = “{ $name }”
pause-note-schedule = Maintenance window “{ $name }”
log-schedule-started = Maintenance window
log-schedule-started-message = “{ $name }”: { $sensors ->
        [one] 1 sensor
       *[other] { $sensors } sensors
    } on { $devices ->
        [one] 1 device
       *[other] { $devices } devices
    } paused
log-schedule-ended = Maintenance window ended
log-schedule-ended-message = “{ $name }”: sensors run again
log-scope-schedule = Schedule
