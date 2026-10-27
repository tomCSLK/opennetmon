# Dependencies like PRTG: objects pause while a sensor they depend on is down or paused.
dependency = Dependency
dependency-heading = Dependency of “{ $name }”
dependency-intro = An object that depends on a sensor is paused while that sensor is down or paused – for example the switches while the ping of the router fails. So you get one mail for the cause instead of one for every sensor behind it. The sensor depended on is always polled.
dependency-hint-group = Applies to all subgroups, devices and sensors of the group that set nothing else.
dependency-hint-device = Applies to all sensors of the device that set nothing else.
dependency-parent = Inherit from the parent object
dependency-sensor = Depend on a sensor
dependency-none = No dependency
dependency-none-hint = Stops the inheritance from above.
dependency-current = currently: { $value }
dependency-current-none = no dependency
dependency-on = “{ $sensor }” ({ $device })
dependency-from-device = from the device
dependency-from-group = from the group “{ $name }”
dependency-choose = Choose a sensor …
dependency-delay = Delay after recovery (seconds)
dependency-delay-hint = How long the objects stay paused once the sensor is fine again, e.g. so that a switch can boot after the router restarted. Empty = inherit ({ $current }).
dependency-master = Master sensor of the device
dependency-master-hint = While it is down or paused, the other sensors of the device are paused; typically the ping sensor. A device has at most one.
dependency-choice-invalid = Please choose one of the options.
dependency-sensor-required = Please choose a sensor.
dependency-sensor-missing = The chosen sensor does not exist (any more).
dependency-cycle = Not possible: a sensor would then depend on itself through the dependencies.
dependency-delay-invalid = The delay must be empty or a whole number from 0 to { $max } seconds.
dependency-depends-on = depends on “{ $sensor }” ({ $device })
dependency-is-master = master sensor of the device
