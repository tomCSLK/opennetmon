overview-all-devices = All devices
overview-ungrouped = Without group
overview-devices-down =
    { $count ->
        [one] { $count } device down
       *[other] { $count } devices down
    }
overview-order-groups = Arrange groups
overview-sensor-status = Sensor status
overview-paused-unknown = Paused / unknown
overview-empty = No devices yet.
overview-empty-hint = Create a device – with an SNMP profile or “No SNMP – ping only”:
group-new = New group
group-edit = Edit group
group-empty = No devices in this group.
device-new = New device
device-edit = Edit device
device-test = Test
device-no-snmp = no SNMP
group-invalid = Invalid group.
group-name-taken = There already is a group with this name on this level.
group-parent-missing = The parent group does not exist.
group-too-deep = Groups can be nested at most 8 levels deep.
group-has-subgroups = The group still contains subgroups.
group-has-devices = The group still contains devices.
group-parent = Parent group
group-top-level = – top level –
group-delete = Delete group
group-delete-hint = A group can only be deleted when it contains neither subgroups nor devices.
group-delete-confirm = Yes, delete this group
name-invalid = The name must be 1 to 64 characters long and must not contain control characters.
host-invalid = Please enter a valid IP address or host name.
polling-port-invalid = The port must be between 1 and 65535.
polling-interval-invalid = The interval must be between 10 and 3600 seconds.
polling-timeout-invalid = The timeout must be between 1 and 60 seconds.
polling-retries-invalid = There may be at most 5 retries.
polling-attempts-too-long = Timeout × (retries + 1) must not exceed the interval.
device-name-taken = A device with this name already exists.
device-group-or-profile-missing = The chosen group or SNMP profile does not exist.
device-profile-missing = Please choose an SNMP profile or “No SNMP”.
polling-port-not-number = Invalid value for the port.
polling-interval-not-number = Invalid value for the interval.
polling-timeout-not-number = Invalid value for the timeout.
polling-retries-not-number = Invalid value for the retries.
device-created-named = Device “{ $name }” created
device-snmp-sensors-left =
    { $count ->
        [one] The device still has an SNMP sensor.
       *[other] The device still has { $count } SNMP sensors.
    } For “No SNMP”, delete them first or keep an SNMP profile.
device-test-connection = Test connection
device-no-profiles = There is no SNMP profile yet – devices can only be monitored by ping.
device-host = IP address or host name
group = Group
group-none = – no group –
profile = SNMP profile
device-ping-only = No SNMP – ping only
device-ping-only-hint = “No SNMP” for devices without SNMP (e.g. routers, printers, cameras): a ping sensor is added right away.
device-polling = Polling
device-port = UDP port
device-interval = Interval (seconds, 10 to 3600)
device-timeout = Timeout per attempt (seconds, 1 to 60)
device-retries = Retries (0 to 5)
device-template = Device template
device-template-after-create = After creating
device-template-hint = Creates the template's sensors right away. “Automatic” picks the template whose device identifier matches the device. Only with an SNMP profile.
device-delete = Delete device
device-delete-hint = Removes the device with all sensors and measurements.
device-delete-confirm = Yes, delete this device
profile-create = Create SNMP profile
device-test-title = Connection test
device-test-heading = Connection test: { $name }
device-test-ok = Connection successful ({ $ms } ms).
device-sys-descr = Device description
device-uptime = Uptime
device-test-again = Test again
device-to-sensors = To the sensors
device-test-limits = The test uses at most a 10 second timeout and one retry.
device-without-snmp = This device has no SNMP set up (ping only). An SNMP profile can be chosen under “Edit device”.

group-move-into-itself = A group cannot be moved into itself or into one of its subgroups.
device-kind = Device type
device-kind-hint = Chooses the icon in the overview.
device-kind-invalid = Unknown device type.
device-kind-generic = Generic
device-kind-router = Router
device-kind-switch = Switch
device-kind-access-point = Access point
device-kind-firewall = Firewall
device-kind-server = Server
device-kind-storage = NAS / storage
device-kind-computer = PC
device-kind-printer = Printer
device-kind-camera = Camera
device-kind-smart-home = Smart home / IoT
device-kind-internet = Internet service
