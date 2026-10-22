overview-all-devices = Alle Geräte
overview-ungrouped = Ohne Gruppe
overview-devices-down =
    { $count ->
        [one] { $count } Gerät mit Ausfall
       *[other] { $count } Geräte mit Ausfall
    }
overview-order-groups = Gruppen anordnen
overview-sensor-status = Sensorstatus
overview-paused-unknown = Pausiert / unbekannt
overview-empty = Noch keine Geräte angelegt.
overview-empty-hint = Lege ein Gerät an – mit SNMP-Profil oder „Kein SNMP – nur Ping“:
group-new = Neue Gruppe
group-edit = Gruppe bearbeiten
group-empty = Keine Geräte in dieser Gruppe.
group-new-sub = Neue Untergruppe
device-new = Neues Gerät
device-edit = Gerät bearbeiten
device-test = Testen
device-no-snmp = kein SNMP
group-invalid = Ungültige Gruppe.
group-name-taken = In dieser Ebene gibt es bereits eine Gruppe mit diesem Namen.
group-parent-missing = Die übergeordnete Gruppe existiert nicht.
group-too-deep = Die Gruppen sind höchstens 8 Ebenen tief verschachtelbar.
group-has-subgroups = Die Gruppe enthält noch Untergruppen.
group-has-devices = Die Gruppe enthält noch Geräte.
group-parent = Übergeordnete Gruppe
group-top-level = – oberste Ebene –
group-delete = Gruppe löschen
group-delete-hint = Löschen ist nur möglich, wenn die Gruppe weder Untergruppen noch Geräte enthält.
group-delete-confirm = Ja, diese Gruppe löschen
name-invalid = Der Name muss 1 bis 64 Zeichen lang sein und darf keine Steuerzeichen enthalten.
host-invalid = Bitte eine gültige IP-Adresse oder einen Hostnamen angeben.
polling-port-invalid = Der Port muss zwischen 1 und 65535 liegen.
polling-interval-invalid = Das Intervall muss zwischen 10 und 3600 Sekunden liegen.
polling-timeout-invalid = Der Timeout muss zwischen 1 und 60 Sekunden liegen.
polling-retries-invalid = Die Anzahl der Wiederholungen darf höchstens 5 betragen.
polling-attempts-too-long = Timeout × (Wiederholungen + 1) darf das Intervall nicht überschreiten.
device-name-taken = Ein Gerät mit diesem Namen existiert bereits.
device-group-or-profile-missing = Die gewählte Gruppe oder das gewählte SNMP-Profil existiert nicht.
device-profile-missing = Bitte ein SNMP-Profil oder „Kein SNMP“ auswählen.
polling-port-not-number = Ungültiger Wert für den Port.
polling-interval-not-number = Ungültiger Wert für das Intervall.
polling-timeout-not-number = Ungültiger Wert für den Timeout.
polling-retries-not-number = Ungültiger Wert für die Wiederholungen.
device-created-named = Gerät „{ $name }“ angelegt
device-snmp-sensors-left =
    { $count ->
        [one] Das Gerät hat noch einen SNMP-Sensor.
       *[other] Das Gerät hat noch { $count } SNMP-Sensoren.
    } Für „Kein SNMP“ diese zuerst löschen oder ein SNMP-Profil behalten.
device-test-connection = Verbindung testen
device-no-profiles = Es gibt noch kein SNMP-Profil – Geräte lassen sich nur per Ping überwachen.
device-host = IP-Adresse oder Hostname
group = Gruppe
group-none = – keine Gruppe –
profile = SNMP-Profil
device-ping-only = Kein SNMP – nur Ping
device-ping-only-hint = „Kein SNMP“ für Geräte ohne SNMP (z. B. Router, Drucker, Kameras): Beim Anlegen kommt gleich ein Ping-Sensor dazu.
device-polling = Abfrage
device-port = UDP-Port
device-interval = Intervall (Sekunden, 10 bis 3600)
device-timeout = Timeout je Versuch (Sekunden, 1 bis 60)
device-retries = Wiederholungen (0 bis 5)
device-template = Gerätevorlage
device-template-after-create = Nach dem Anlegen
device-template-hint = Legt die Sensoren der Vorlage gleich mit an. „Automatisch“ wählt die Vorlage, deren Gerätekennung zum Gerät passt. Gilt nur mit SNMP-Profil.
device-delete = Gerät löschen
device-delete-hint = Entfernt das Gerät mit allen Sensoren und Messwerten.
device-delete-confirm = Ja, dieses Gerät löschen
profile-create = SNMP-Profil anlegen
device-test-title = Verbindungstest
device-test-heading = Verbindungstest: { $name }
device-test-ok = Verbindung erfolgreich ({ $ms } ms).
device-sys-descr = Gerätebeschreibung
device-uptime = Laufzeit
device-test-again = Erneut testen
device-to-sensors = Zu den Sensoren
device-test-limits = Der Test nutzt höchstens 10 Sekunden Timeout und eine Wiederholung.
device-without-snmp = Für dieses Gerät ist kein SNMP eingerichtet (nur Ping). Ein SNMP-Profil lässt sich unter „Gerät bearbeiten“ wählen.

group-move-into-itself = Eine Gruppe kann nicht in sich selbst oder in eine ihrer Untergruppen verschoben werden.
device-kind = Gerätetyp
device-kind-hint = Bestimmt das Symbol in der Übersicht.
device-kind-invalid = Unbekannter Gerätetyp.
device-kind-generic = Allgemein
device-kind-router = Router
device-kind-switch = Switch
device-kind-access-point = Access Point
device-kind-firewall = Firewall
device-kind-server = Server
device-kind-storage = NAS / Speicher
device-kind-computer = PC
device-kind-printer = Drucker
device-kind-camera = Kamera
device-kind-smart-home = Smart Home / IoT
device-kind-internet = Internetdienst
