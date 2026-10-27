# Reports (availability)
nav-reports = Reports
report-subtitle = Availability and downtime · { $period }
report-view = View
report-view-devices = Devices
report-view-sensors = Sensors
report-period = Period
report-period-24h = 24 h
report-period-7d = 7 days
report-period-30d = 30 days
report-show = Show
report-csv = Download CSV
report-hint = Available = OK or warning, unavailable = down. Paused time, maintenance windows and time without polls do not count. A device follows its master sensor, else its ping sensor, else the mean of its sensors. Months in the time zone of the server; counted per hour since the update to this version.
report-empty = No devices or sensors yet.
report-availability = Availability
report-downtime = Downtime
report-no-data = no data
report-counted = counted { $time }
report-basis-master = by master sensor “{ $sensor }”
report-basis-ping = by “{ $sensor }”
report-basis-mean = { $count ->
        [one] mean of 1 sensor
       *[other] mean of { $count } sensors
    }
report-availability-of = Availability { $period }
report-downtime-of = Down { $time }
month-1 = January
month-2 = February
month-3 = March
month-4 = April
month-5 = May
month-6 = June
month-7 = July
month-8 = August
month-9 = September
month-10 = October
month-11 = November
month-12 = December
