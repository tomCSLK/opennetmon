catalog-tp-poe-ports = PoE per port (TP-Link)
catalog-tp-poe-total = PoE total (TP-Link)
catalog-tp-cpu = CPU utilization (TP-Link)
catalog-tp-memory = Memory utilization (TP-Link)
catalog-hr-cpu = CPU load (HOST-RESOURCES-MIB)
catalog-hr-storage = Memory and storage (HOST-RESOURCES-MIB)
catalog-ent-sensors = Hardware sensors: temperature, fans, voltage (ENTITY-SENSOR-MIB)
catalog-poe-power = PoE power
catalog-poe-status = PoE status
catalog-poe-current = PoE current
catalog-poe-voltage = PoE voltage
catalog-poe-class = PoE class
catalog-poe-consumption = PoE consumption total
catalog-poe-remain = PoE remaining
catalog-poe-limit = PoE budget
catalog-cpu-1min = CPU 1 min
catalog-cpu-5s = CPU 5 s
catalog-cpu-5min = CPU 5 min
catalog-memory = Memory
catalog-cpu-load = CPU load
catalog-storage-used = Used
catalog-storage-size = Size
catalog-sensor-value = Reading
catalog-sensor-status = Sensor status

values-counter-reading = counter reading
values-truncated = Cut to { $count } values.
values-row-without-table = The table of this row is not known.
values-not-a-table = “{ $name }” is of kind { $kind } – please enter a table or group (single values via “Add sensor”).
values-name-missing = Every selected value needs a name.
values-select-one = Please select at least one value.

values-heading = Values of { $device }
values-catalog = Known tables (PoE, CPU, memory, hardware sensors …) from the loaded MIBs
values-object = Table or group “{ $name }”
values-object-placeholder = Another table or group, e.g. tpPoePortConfigTable or IF-MIB::ifXTable (empty: known tables)
values-object-label = Table or group
values-search = Search
values-hint = You find names under { $link }. New sensors are created paused and only polled once resumed.
values-none = No values found.
values-none-catalog = The device answers none of the known tables. Are the vendor's MIB files missing? Or enter a specific table above.
values-none-object = The device returns no numbers for this table.
count-values =
    { $count ->
        [one] { $count } value
       *[other] { $count } values
    }
values-create = Create
values-sensor-name = Sensor name
values-now = Current value
values-exists = exists
values-create-named = Create { $name }
values-activate = Start now (otherwise create paused and resume later on the device page)
values-create-selected = Create the selected values as sensors
