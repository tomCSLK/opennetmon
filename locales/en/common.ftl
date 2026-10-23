# Common texts: navigation, roles, frequent words, error pages, language, messages after actions.
# German is the source language; every other language needs the same messages with the same variables.

## Navigation and page frame

nav-menu = Menu
nav-main = Main navigation
nav-monitoring = Monitoring
nav-overview = Overview
nav-settings = Settings
nav-credentials = SNMP profiles
nav-users = Users
nav-mail = E-mail
nav-templates = Device templates
nav-mibs = MIB files
nav-account = My account
nav-theme = Theme
nav-theme-toggle = Toggle theme
nav-logout = Sign out
source-code = Source code

## Roles

role = Role
role-admin = Administrator
role-viewer = Viewer
role-admin-option = Administrator – may change everything
role-viewer-option = Viewer – read only

## Frequent words

save = Save
edit = Edit
delete = Delete
username = User name
password = Password
password-repeat = Repeat password
password-repeat-short = Repeat
password-new = New password
password-new-repeat = Repeat new password
confirm-delete = Please confirm the deletion.
refresh = Refresh
refresh-auto = Auto
refresh-auto-title = Reload the page every 30 seconds (waits while you are entering something)

## Error pages

error-forbidden = Access denied
error-not-found = Not found
error-internal = Internal error
error-code = Error { $status }
error-heading = That did not work
error-heading-input = Input not possible
error-heading-not-found = Page not found
error-heading-forbidden = Access denied
error-heading-internal = Internal error
error-not-found-text = This page does not exist (any more). Perhaps the entry was deleted.
error-bad-request = The data could not be processed. Please check what you entered and try again.
error-method = This action is not possible here.
error-internal-text = An error occurred while processing. The details are in the server log.
error-back = Back

## Language

language = Language
language-browser = Same as the browser
language-invalid = This language does not exist.

## Messages after actions

flash-group-created = Group created.
flash-group-updated = Group saved.
flash-group-deleted = Group deleted.
flash-device-created = Device created.
flash-device-created-ping = Device created without SNMP; the ping sensor is active.
flash-device-updated = Device saved.
flash-device-deleted = Device deleted.
flash-sensor-created = Sensor created.
flash-sensors-added = Sensors created.
flash-sensor-deleted = Sensor deleted.
flash-sensor-paused = Sensor paused.
flash-sensor-resumed = Sensor resumed.
flash-device-paused = Device paused.
flash-device-resumed = Device resumed.
flash-group-paused = Group paused.
flash-group-resumed = Group resumed.
flash-values-added-inactive = Sensors created – they are still paused; resume them below under “Paused sensors”.
flash-values-added-active = Sensors created and started.
flash-sensors-activated = Sensors resumed.
flash-sensors-deleted = Sensors deleted.
flash-thresholds-saved = Thresholds saved.
flash-sensor-renamed = Sensor renamed.
flash-interval-saved = Polling interval saved.
flash-states-saved = Status value ratings saved. They apply from the next poll.
flash-states-reset = Status value ratings removed.
flash-order-saved = Order saved.
flash-order-reset = Order reset: sorted automatically again.
flash-template-created = Template saved. Use it when creating a device or with “Apply template”.
flash-template-saved = Template saved.
flash-template-deleted = Template deleted.
flash-user-created = User created.
flash-user-role-saved = Role saved.
flash-user-language-saved = Language saved.
flash-user-password-saved = Password set.
flash-user-deleted = User deleted.
flash-account-password-saved = Password changed. Your other sessions were signed out.
flash-language-saved = Language saved.
flash-mail-saved = E-mail settings saved.
flash-mib-deleted = MIB file deleted.
flash-profile-created = Profile created.
flash-profile-updated = Profile saved.
flash-profile-deleted = Profile deleted.

status-up = OK
status-warning = Warning
status-down = Down
status-unknown = Unknown
status-paused = Paused
no-sensors = no sensors
sensors = Sensors
name = Name
cancel = Cancel
order = Order
edit-named = Edit { $name }
every-seconds = every { $seconds } s
count-devices =
    { $count ->
        [one] { $count } device
       *[other] { $count } devices
    }
count-sensors =
    { $count ->
        [one] { $count } sensor
       *[other] { $count } sensors
    }
count-up = { $count } OK
count-unknown = { $count } unknown
count-inactive = { $count } paused
count-warning = { $count } warning
count-down = { $count } down
duration-minutes = { $minutes } min
duration-hours-minutes = { $hours } h { $minutes } min
duration-days-hours-minutes =
    { $days ->
        [one] { $days } day
       *[other] { $days } days
    } { $hours } h { $minutes } min
event-first-poll = first poll: { $status }
snmp-timeout = No answer within the timeout. Possible causes: device or port not reachable, SNMP not enabled, wrong community or wrong SNMPv3 credentials.
snmp-auth = Authentication failed. User, password, authentication or privacy protocol do not match the device.
snmp-unreachable = The device is not reachable or the SNMP port is closed.
snmp-request = The request could not be built: { $detail }
snmp-protocol = Unexpected answer from the device: { $detail }

profile-summary-user = user { $user }

duration-minutes-seconds = { $minutes } min { $seconds } s
channel-in = Inbound
channel-out = Outbound
channel-rtt = Response time
channel-uptime = Uptime
channel-oper-status = Status
channel-value = Value
threshold-not-a-number = A threshold must be a number.
threshold-not-finite = A threshold must be a finite number.

unknown-action = Unknown action.
inactive = paused

status = Status
sensor = Sensor
message = Message
actions = Actions

active = active

backup-path-not-utf8 = The backup path is not valid UTF-8.
backup-exists = { $path } already exists; an existing backup is never overwritten.
