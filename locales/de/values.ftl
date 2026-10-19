catalog-tp-poe-ports = PoE je Port (TP-Link)
catalog-tp-poe-total = PoE gesamt (TP-Link)
catalog-tp-cpu = CPU-Auslastung (TP-Link)
catalog-tp-memory = Speicher-Auslastung (TP-Link)
catalog-hr-cpu = CPU-Last (HOST-RESOURCES-MIB)
catalog-hr-storage = Speicher und Datenträger (HOST-RESOURCES-MIB)
catalog-ent-sensors = Hardware-Sensoren: Temperatur, Lüfter, Spannung (ENTITY-SENSOR-MIB)
catalog-poe-power = PoE-Leistung
catalog-poe-status = PoE-Status
catalog-poe-current = PoE-Strom
catalog-poe-voltage = PoE-Spannung
catalog-poe-class = PoE-Klasse
catalog-poe-consumption = PoE-Verbrauch gesamt
catalog-poe-remain = PoE-Reserve
catalog-poe-limit = PoE-Budget
catalog-cpu-1min = CPU 1 Min.
catalog-cpu-5s = CPU 5 s
catalog-cpu-5min = CPU 5 Min.
catalog-memory = Speicher
catalog-cpu-load = CPU-Last
catalog-storage-used = Belegt
catalog-storage-size = Größe
catalog-sensor-value = Messwert
catalog-sensor-status = Sensorstatus

values-counter-reading = Zählerstand
values-truncated = Gekürzt auf { $count } Werte.
values-row-without-table = Die Tabelle zu dieser Zeile ist nicht bekannt.
values-not-a-table = „{ $name }“ ist vom Typ { $kind } – bitte eine Tabelle oder Gruppe angeben (einzelne Werte über „Sensor hinzufügen“).
values-name-missing = Jeder ausgewählte Wert braucht einen Namen.
values-select-one = Bitte mindestens einen Wert auswählen.

values-heading = Werte von { $device }
values-catalog = Bekannte Tabellen (PoE, CPU, Speicher, Hardware-Sensoren …) aus den geladenen MIBs
values-object = Tabelle bzw. Gruppe „{ $name }“
values-object-placeholder = Andere Tabelle oder Gruppe, z. B. tpPoePortConfigTable oder IF-MIB::ifXTable (leer: bekannte Tabellen)
values-object-label = Tabelle oder Gruppe
values-search = Durchsuchen
values-hint = Namen findest du unter { $link }. Neue Sensoren werden inaktiv angelegt und erst nach dem Aktivieren abgefragt.
values-none = Keine Werte gefunden.
values-none-catalog = Das Gerät beantwortet keine der bekannten Tabellen. Fehlen vielleicht die MIB-Dateien des Herstellers? Oder eine bestimmte Tabelle oben eingeben.
values-none-object = Das Gerät liefert für diese Tabelle keine Zahlenwerte.
count-values =
    { $count ->
        [one] { $count } Wert
       *[other] { $count } Werte
    }
values-create = Anlegen
values-sensor-name = Sensorname
values-now = Wert jetzt
values-exists = vorhanden
values-create-named = { $name } anlegen
values-activate = Sofort aktivieren (sonst inaktiv anlegen und später auf der Geräteseite aktivieren)
values-create-selected = Ausgewählte als Sensoren anlegen
