# Dependencies like PRTG: objects pause while a sensor they depend on is down or paused.
dependency = Abhängigkeit
dependency-heading = Abhängigkeit für „{ $name }“
dependency-intro = Hängt ein Objekt von einem Sensor ab, ist es pausiert, solange dieser Sensor ausfällt oder pausiert ist – zum Beispiel die Switche, wenn der Ping des Routers ausfällt. So kommt eine Mail für die Ursache statt einer für jeden Sensor dahinter. Der Sensor, von dem es abhängt, wird immer weiter abgefragt.
dependency-hint-group = Gilt für alle Untergruppen, Geräte und Sensoren der Gruppe, die nichts anderes einstellen.
dependency-hint-device = Gilt für alle Sensoren des Geräts, die nichts anderes einstellen.
dependency-parent = Vom übergeordneten Objekt übernehmen
dependency-sensor = Von einem Sensor abhängig
dependency-none = Keine Abhängigkeit
dependency-none-hint = Beendet die Vererbung von oben.
dependency-current = derzeit: { $value }
dependency-current-none = keine Abhängigkeit
dependency-on = „{ $sensor }“ ({ $device })
dependency-from-device = vom Gerät
dependency-from-group = von der Gruppe „{ $name }“
dependency-choose = Sensor wählen …
dependency-delay = Verzögerung nach der Rückkehr (Sekunden)
dependency-delay-hint = So lange bleiben die Objekte noch pausiert, wenn der Sensor wieder in Ordnung ist, z. B. damit ein Switch nach dem Neustart des Routers hochfahren kann. Leer = übernehmen ({ $current }).
dependency-master = Master-Sensor für das Gerät
dependency-master-hint = Fällt er aus oder ist er pausiert, sind die übrigen Sensoren des Geräts pausiert; typisch ist der Ping-Sensor. Je Gerät gibt es höchstens einen.
dependency-choice-invalid = Bitte eine der Möglichkeiten wählen.
dependency-sensor-required = Bitte einen Sensor wählen.
dependency-sensor-missing = Den gewählten Sensor gibt es nicht (mehr).
dependency-cycle = Das geht nicht: Ein Sensor hinge dann über die Abhängigkeiten von sich selbst ab.
dependency-delay-invalid = Die Verzögerung muss leer sein oder eine ganze Zahl von 0 bis { $max } Sekunden.
dependency-depends-on = abhängig von „{ $sensor }“ ({ $device })
dependency-is-master = Master-Sensor des Geräts
