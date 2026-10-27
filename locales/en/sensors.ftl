thresholds-not-finite = Thresholds must be finite numbers.
thresholds-down-above-below-warn = The upper down limit must not be below the upper warning limit.
thresholds-down-below-above-warn = The lower down limit must not be above the lower warning limit.
thresholds-upper-below-lower = The upper limit must not be below the lower limit.
sensor-bad-value = Unusable value: { $detail }
sensor-no-session = No SNMP session to the device possible.
sensor-unreachable = Not reachable: { $detail }
assess-no-status-value = No status value received
assess-state-down = { $state } counts as down
assess-state-warning = { $state } counts as a warning
assess-state-unknown = Unknown status value { $value }
assess-interface-up = Interface is up
assess-interface-not-up = Interface is not up (ifOperStatus = { $status })
assess-above-warn = { $channel }: { $value } is above the warning limit { $limit }
assess-below-warn = { $channel }: { $value } is below the warning limit { $limit }
assess-above-down = { $channel }: { $value } is above the down limit { $limit }
assess-below-down = { $channel }: { $value } is below the down limit { $limit }
assess-ok = OK

poll-no-snmp = This device has no SNMP set up (only ping, services and the values of this server).
poll-profile-missing = The device's SNMP profile no longer exists.
poll-credentials-unreadable = The SNMP credentials cannot be read.

sensor-describe-traffic = Interface traffic (index { $index })
sensor-describe-status = Interface status (index { $index })
oid-mode-gauge = Value
oid-mode-counter = Counter, rate/s
oid-detail-factor = , factor { $factor }
oid-detail-unit = , unit { $unit }
sensor-if-index-invalid = The interface index must be between 1 and 2147483647.
sensor-factor-invalid = The factor must be a number other than 0.
sensor-unit-invalid = The unit must be at most 24 characters long and must not contain control characters.
oid-invalid = The OID is invalid (at least 2 numbers, the first 0 to 2, at most 128 parts).
oid-not-numeric = The OID is invalid. A numeric OID like 1.3.6.1.2.1.1.3.0 is expected.
range-hour = 1 hour
range-day = 24 hours
range-week = 7 days
range-month = 30 days

channel-error = { $channel }: { $message }
states-none = This sensor has no named status values.
states-choose = { $value }: please choose a rating.

sensors-ping-only = no SNMP
sensors-find-values = Find values
sensors-find-interfaces = Find interfaces
count-active = { $count } active
order-change = Change order
sensors-none-yet = No sensors yet.
sensors-none-active = No active sensors.
sensors-empty-hint = “Find interfaces” takes ports from the device; below you can add ping, uptime, custom OIDs and values of this server.
sensors-empty-hint-ping = You can add a ping sensor and values of this server below.
sensors-inactive-title = Paused sensors ({ $count })
sensors-inactive-hint = They are not polled and send no notifications until they are resumed or their pause ends.
select-named = Select { $name }
sensors-activate-selected = Resume selected
sensors-activate-all = Resume all
sensors-delete-selected = Delete selected
sensors-template-hint = A template adds missing sensors; you can also save a new template from this device.
sensors-template-used = Set up with “{ $name }”.
template-apply = Apply template
template-save-as = Save as template
sensors-add = Add sensor
sensors-type = Type
sensors-type-ping = Ping (response time)
sensors-oid-only = Only for “Custom OID”
sensors-type-oid = Custom OID
sensors-type-server = This server (CPU, memory, file systems …)
sensors-name-placeholder = empty: Ping, Uptime or the MIB name
sensors-oid-or-name = OID or MIB name
sensors-oid-placeholder = e.g. tpSysMonitorCpu5Seconds.1, IF-MIB::ifInErrors.3 or 1.3.6.1.2.1.1.3.0
sensors-oid-hint = Names from the loaded MIBs are suggested while typing ({ $link }). Scalars get “.0” automatically; for table columns append the index.
sensors-mode = Evaluation
sensors-mode-auto = Automatic (from the MIB, otherwise value)
sensors-mode-gauge = Value as is (gauge)
sensors-mode-counter = Counter, change per second
sensors-scale = Factor
sensors-unit = Unit (optional)
sensors-unit-placeholder = empty: from the MIB
sensors-name-hint = Without a name the sensor is named after the MIB object (e.g. “tpSysMonitorCpu5Seconds.1”).
sensors-no-snmp-hint = This device has no SNMP profile, so only ping, services (HTTP, TCP, DNS, TLS) and the values of this server are available. For uptime, interfaces and custom values choose an SNMP profile under “Edit device”.
add = Add

discover-heading = Interfaces of { $device }
discover-again = Search again
back-to-sensors = Back to the sensors
discover-none = The device reports no interfaces.
discover-found =
    { $count ->
        [one] { $count } interface found
       *[other] { $count } interfaces found
    }
discover-hint = Preselected: active ports (Ethernet, Wi-Fi). Existing sensors are locked.
description = Description
discover-state = State
discover-speed = Speed
discover-traffic-for = Traffic sensor for { $name }
discover-status-for = Status sensor for { $name }
discover-create = Create sensors

sensors-mode-missing = Please choose the evaluation (automatic, value or counter).
sensors-scale-not-number = The factor must be a number.
sensors-type-missing = Please choose a sensor type.
sensors-name-missing = Please enter a name (the OID is not known in any loaded MIB).
form-too-large = The form is too large.
sensors-select-interface = Please select at least one interface.
sensors-select-sensor = Please select at least one sensor.

detail-last-poll = last poll
detail-range = Time range
detail-latest = Latest values
detail-no-value = no value yet
detail-no-samples = No values in the chosen time range yet.
detail-chart-needs-js = The chart needs JavaScript.
detail-samples = Values
# How far back the table of values reaches ($minutes: 15, 30, 60, 360, 1440 or 10080).
detail-span = { $minutes ->
    [60] last hour
    [360] last 6 hours
    [1440] last 24 hours
    [10080] last 7 days
   *[other] last { $minutes } minutes
}
detail-table-needs-js = The table needs JavaScript.
detail-new-name = New name
detail-rename = Rename
detail-rename-hint = At most 64 characters, unique per device. Values, chart, thresholds and status are kept.
detail-interval = Polling interval
detail-interval-seconds = Interval in seconds
detail-interval-own = own interval
detail-interval-hint = Leave empty for the device's interval ({ $seconds } s). 10 to 3600 seconds; shorter intervals mean more values and a larger database.
detail-interval-invalid = Please enter a whole number of seconds or leave the field empty.
detail-states = Status values
detail-states-rated = rated
detail-states-stored = Each value decides the sensor status. Values not listed here count as a warning.
detail-states-interface = Default: only “up” is fine, everything else is a failure. You can change this per value here, e.g. “down” only as a warning.
detail-states-unrated = Not rated yet – the value does not change the sensor status. Below is a suggestion from the names of the values; only “Save” switches the rating on.
detail-value = Value
detail-rating = Rating
detail-rating-for = Rating for { $value }
detail-states-restore = Restore default
detail-states-off = Switch rating off
detail-thresholds = Thresholds
detail-thresholds-hint = A value triggers when it goes above (“above”) or below (“below”) the limit. “Down” and “Warning” apply at once. Empty fields mean: no limit.
detail-in-unit = (in { $unit })
threshold-warn-above = Warning above
threshold-warn-below = Warning below
threshold-down-above = Down above
threshold-down-below = Down below
detail-thresholds-save = Save thresholds
detail-thresholds-admins = Only administrators can change the thresholds.
detail-events = Recent status changes
detail-no-events = No events yet.
detail-when = Time
detail-change = Change

sensor-name-taken = A sensor with this name already exists on this device.
sensor-device-missing = The chosen device does not exist.
states-rating-invalid = Invalid rating.
states-label-invalid = Invalid name of a status value.
sensors-current = Current
sensor-settings = Settings
sensor-settings-intro = What the sensor checks. Type, name, limits and past values stay; the next poll uses the new settings.
sensor-delete = Delete sensor
sensor-delete-hint = Removes the sensor with all its measurements and history. This cannot be undone.
sensor-delete-confirm = Yes, delete this sensor
sensors-trend-of = Last hour: { $name }
