# Audit log
nav-audit = Changes
audit-subtitle = Who changed what and when, plus sign-ins and sign-outs · administrators only
audit-empty = No entries yet.
audit-filter = Filter entries …
audit-filtered-user = user { $user }
audit-when = Time
audit-user = User
audit-action = Action
audit-object = Object
audit-detail = Change

audit-action-created = created
audit-action-changed = changed
audit-action-deleted = deleted
audit-action-signed-in = signed in
audit-action-sign-in-failed = sign-in failed
audit-action-signed-out = signed out

audit-kind-group = Group
audit-kind-device = Device
audit-kind-sensor = Sensor
audit-kind-profile = SNMP profile
audit-kind-user = User
audit-kind-template = Device template
audit-kind-mib = MIB file
audit-kind-mail = Email settings
audit-kind-schedule = Schedule
audit-kind-session = Sign-in

audit-change = { $field }: { $old } → { $new }
audit-value = { $field }: { $value }
audit-secret-changed = { $field } changed
audit-limit = { $channel }, { $limit }: { $old } → { $new }
audit-yes = yes
audit-no = no
audit-none = none
audit-inherited = inherit
audit-not-rated = not rated
audit-unknown-user = unknown user name
audit-wrong-password = wrong password
audit-signed-out-idle = after a time without activity
audit-template-applied =
    { $count ->
        [one] template “{ $template }” applied, one sensor created
       *[other] template “{ $template }” applied, { $count } sensors created
    }
audit-order-saved = order saved
audit-order-reset = automatic order
audit-order-saved-top = order of the top level saved
audit-order-reset-top = automatic order of the top level
audit-window-added = window added: { $window }
audit-window-deleted = window deleted: { $window }

audit-field-name = Name
audit-field-host = Address
audit-field-group = Group
audit-field-parent = Parent group
audit-field-profile = SNMP profile
audit-field-kind = Device type
audit-field-port = Port
audit-field-interval = Interval (s)
audit-field-timeout = Timeout (s)
audit-field-retries = Retries
audit-field-device = Device
audit-field-snmp = SNMP
audit-field-username = User name
audit-field-password = Password
audit-field-community = Community
audit-field-auth-password = Authentication password
audit-field-priv-password = Privacy password
audit-field-role = Role
audit-field-language = Language
audit-field-stay-signed-in = Stay signed in
audit-field-object-id = sysObjectID
audit-field-modules = Modules
audit-field-enabled = Enabled
audit-field-smtp-host = SMTP server
audit-field-security = Encryption
audit-field-from = Sender
audit-field-recipients = Recipients
audit-field-notify-warnings = Report warnings
audit-field-dependency = Dependency
audit-field-delay = Delay (s)
audit-field-master = Master sensor
audit-field-schedule = Schedule
