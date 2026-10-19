template-named-error = “{ $name }”: { $message }
template-channel-invalid = invalid channel.
template-states-invalid = invalid status rating.
template-version-unknown = Unknown template version { $version }.
template-too-many = A template holds at most { $count } entries.
template-interface-invalid = invalid interface name.
template-custom-values =
    { $count ->
        [one] { $count } custom value
       *[other] { $count } custom values
    }
template-empty = empty
template-ports-same =
    { $count ->
        [one] { $count } specific port
       *[other] { $count } specific ports
    }
template-ports-up = active ports
template-ports-all = all ports
template-interfaces-unread = The interfaces of the device could not be read, so “the same ports” is not possible.
template-would-be-empty = The template would be empty. Please take over at least one sensor.
template-skipped-unread = { $what }: the interfaces were not read.
template-skipped-interface = { $name }: the device has no interface “{ $interface }”.
template-skipped-oid = { $name }: the device returns no value for { $oid }.

template-with-thresholds-and-states = with thresholds and status rating
template-with-thresholds = with thresholds
template-with-states = with status rating
template-rule-port = { $what } on “{ $interface }”
template-rule-each-port = one sensor per port “{ $what } <port name>”
template-oid-counter = counter
template-oid-value = value
template-traffic = Traffic
template-interface-status = Interface status
template-result-created =
    { $count ->
        [one] { $count } sensor created
       *[other] { $count } sensors created
    }
template-result-present = , { $count } already present
template-result-skipped = , { $count } skipped
template-no-match = No template matches the device identification { $oid }. A template can also be chosen by hand under “Apply template”.
template-no-object-id = The device reports no identification, so no template matches. A template can also be chosen by hand under “Apply template”.
template-choose = Please choose a template.
template-applied-to = Template applied to { $device }
template-option-none = None – only create the device
template-option-auto = Detect automatically from the device identification

templates-subtitle = Transfer the sensors of a fully set-up device to other devices
templates-none = No templates yet.
templates-none-hint = Set up a device completely and choose “Save as template” under “Device template” on its sensor page.
template = Template
template-contents = Contents
template-object-id = Device identification
template-from = from { $device }
template-by-hand-only = – (by hand only)
templates-how = How templates work
templates-how-save = “Save as template” on the sensor page of a device takes over ping, uptime, interface sensors and custom values (e.g. PoE, CPU) with their names, state (active/inactive), factor, unit and thresholds.
templates-how-apply = When a device is created, or later with “Apply template”, only missing sensors are added; values the device does not deliver are skipped.
templates-how-detect = The device identification (sysObjectID) decides which template is suggested automatically.
template-created-from = created from { $device }
count-entries =
    { $count ->
        [one] { $count } entry
       *[other] { $count } entries
    }
template-rule = Rule
template-change-hint = To change the contents, set up a device accordingly and save a new template from it.
template-name-detection = Name and detection
template-object-id-label = Device identification (sysObjectID)
template-object-id-placeholder = empty: only chosen by hand
template-detection-hint = Devices with exactly this identification get the template suggested when created (“detect automatically”) and under “Apply template”.
template-delete-hint = Devices that were set up with this template keep their sensors.
template-delete = Delete template
template-from-device = Template from { $device }
template-new-subtitle = What you take over, opennetmon later creates on other devices.
template-read-partly = The device could not be read completely: { $message } The device identification and “the same ports” are therefore missing.
template-name = Template name
template-new-detection-hint = Devices with the same identification (same model) get this template suggested automatically.
template-no-sensors-of-kind = This device has no { $kind } sensors.
template-on-device = On the device:
template-choice-up = all active ports of the target device
template-choice-all = all ports, inactive ones too
template-choice-same = the same ports (by port name, with the sensor names from here)
template-choice-skip = do not take over
template-other-sensors = Other sensors
template-no-other-sensors = No ping, uptime or custom sensors.
template-take = Take over
template-take-named = Take over { $name }
template-save = Save template
template-new-hint = Names, state (active/inactive), factor, unit and thresholds are taken over. Custom values (e.g. PoE per port) are created with the same OID; if a target device returns no value for it, the sensor is skipped.
template-identification = Identification
template-object-id-unread = The device identification could not be read: { $message }
templates-none-apply = First save a fully set-up device as a template.
template-detected = Matching the device identification: { $name }.
template-apply-hint = Only missing sensors are created. Existing sensors stay as they are; nothing is deleted.
template-apply-button = Apply
template-named = Template “{ $name }”
template-not-applied = The template was not applied. { $message }
template-result-created-title = Created
template-result-skipped-title = Skipped
to-sensors = To the sensors
to-overview = To the overview

template-object-id-invalid = The device identification must be a numeric OID, e.g. 1.3.6.1.4.1.11863.1.1.11.
template-name-taken = A template with this name already exists.
