thresholds-not-finite = Schwellwerte müssen endliche Zahlen sein.
thresholds-down-above-below-warn = Die obere Ausfallgrenze darf nicht unter der oberen Warngrenze liegen.
thresholds-down-below-above-warn = Die untere Ausfallgrenze darf nicht über der unteren Warngrenze liegen.
thresholds-upper-below-lower = Die obere Grenze darf nicht unter der unteren Grenze liegen.
sensor-bad-value = Unbrauchbarer Messwert: { $detail }
sensor-no-session = Keine SNMP-Sitzung zum Gerät möglich.
sensor-unreachable = Nicht erreichbar: { $detail }
assess-no-status-value = Kein Statuswert erhalten
assess-state-down = { $state } gilt als Ausfall
assess-state-warning = { $state } gilt als Warnung
assess-state-unknown = Unbekannter Statuswert { $value }
assess-interface-up = Interface ist up
assess-interface-not-up = Interface ist nicht up (ifOperStatus = { $status })
assess-above-warn = { $channel }: { $value } liegt über der Warngrenze { $limit }
assess-below-warn = { $channel }: { $value } liegt unter der Warngrenze { $limit }
assess-above-down = { $channel }: { $value } liegt über der Ausfallgrenze { $limit }
assess-below-down = { $channel }: { $value } liegt unter der Ausfallgrenze { $limit }
assess-ok = OK

poll-no-snmp = Für dieses Gerät ist kein SNMP eingerichtet (nur Ping).
poll-profile-missing = Das SNMP-Profil des Geräts existiert nicht mehr.
poll-credentials-unreadable = Die SNMP-Zugangsdaten sind nicht lesbar.

sensor-describe-traffic = Interface-Traffic (Index { $index })
sensor-describe-status = Interface-Status (Index { $index })
oid-mode-gauge = Wert
oid-mode-counter = Zähler, Rate/s
oid-detail-factor = , Faktor { $factor }
oid-detail-unit = , Einheit { $unit }
sensor-if-index-invalid = Der Interface-Index muss zwischen 1 und 2147483647 liegen.
sensor-factor-invalid = Der Faktor muss eine Zahl ungleich 0 sein.
sensor-unit-invalid = Die Einheit darf höchstens 24 Zeichen lang sein und keine Steuerzeichen enthalten.
oid-invalid = Die OID ist ungültig (mindestens 2 Zahlen, erste 0 bis 2, höchstens 128 Stellen).
oid-not-numeric = Die OID ist ungültig. Erwartet wird eine numerische OID wie 1.3.6.1.2.1.1.3.0.
range-hour = 1 Stunde
range-day = 24 Stunden
range-week = 7 Tage
range-month = 30 Tage

channel-error = { $channel }: { $message }
states-none = Dieser Sensor hat keine benannten Statuswerte.
states-choose = { $value }: Bitte eine Bewertung wählen.

sensors-ping-only = kein SNMP, nur Ping
sensors-find-values = Werte suchen
sensors-find-interfaces = Interfaces suchen
count-active = { $count } aktiv
order-change = Reihenfolge ändern
sensors-none-yet = Noch keine Sensoren.
sensors-none-active = Keine aktiven Sensoren.
sensors-empty-hint = Mit „Interfaces suchen“ lassen sich Ports aus dem Gerät übernehmen, unten lassen sich Ping, Uptime, eigene OIDs und Werte dieses Servers anlegen.
sensors-empty-hint-ping = Unten lassen sich ein Ping-Sensor und Werte dieses Servers anlegen.
sensors-inactive-title = Pausierte Sensoren ({ $count })
sensors-inactive-hint = Werden nicht abgefragt und lösen keine Benachrichtigung aus, bis sie fortgesetzt werden oder ihre Pause abläuft.
select-named = { $name } auswählen
sensors-activate-selected = Ausgewählte fortsetzen
sensors-activate-all = Alle fortsetzen
sensors-delete-selected = Ausgewählte löschen
sensors-template-hint = Eine Vorlage legt fehlende Sensoren an; aus diesem Gerät lässt sich auch eine neue Vorlage speichern.
sensors-template-used = Eingerichtet mit „{ $name }“.
template-apply = Vorlage anwenden
template-save-as = Als Vorlage speichern
sensors-add = Sensor hinzufügen
sensors-type = Typ
sensors-type-ping = Ping (Antwortzeit)
sensors-oid-only = Nur für „Eigene OID“
sensors-type-oid = Eigene OID
sensors-type-server = Dieser Server (CPU, Speicher, Dateisysteme …)
sensors-name-placeholder = leer: Ping, Uptime bzw. MIB-Name
sensors-oid-or-name = OID oder MIB-Name
sensors-oid-placeholder = z. B. tpSysMonitorCpu5Seconds.1, IF-MIB::ifInErrors.3 oder 1.3.6.1.2.1.1.3.0
sensors-oid-hint = Namen aus den geladenen MIBs werden beim Tippen vorgeschlagen ({ $link }). Einzelwerte bekommen automatisch „.0“, bei Tabellenspalten den Index anhängen.
sensors-mode = Auswertung
sensors-mode-auto = Automatisch (aus der MIB, sonst Wert)
sensors-mode-gauge = Wert unverändert (Gauge)
sensors-mode-counter = Zähler, Änderung pro Sekunde (Counter)
sensors-scale = Faktor
sensors-unit = Einheit (optional)
sensors-unit-placeholder = leer: aus der MIB
sensors-name-hint = Ohne Namen heißt der Sensor wie das MIB-Objekt (z. B. „tpSysMonitorCpu5Seconds.1“).
sensors-no-snmp-hint = Dieses Gerät hat kein SNMP-Profil, daher gibt es nur Ping und die Werte dieses Servers. Für Uptime, Interfaces und eigene Werte unter „Gerät bearbeiten“ ein SNMP-Profil wählen.
add = Hinzufügen

discover-heading = Interfaces von { $device }
discover-again = Erneut suchen
back-to-sensors = Zurück zu den Sensoren
discover-none = Das Gerät meldet keine Interfaces.
discover-found =
    { $count ->
        [one] { $count } Interface gefunden
       *[other] { $count } Interfaces gefunden
    }
discover-hint = Vorausgewählt: aktive Ports (Ethernet, WLAN). Vorhandene Sensoren sind gesperrt.
description = Beschreibung
discover-state = Zustand
discover-speed = Tempo
discover-traffic-for = Traffic-Sensor für { $name }
discover-status-for = Status-Sensor für { $name }
discover-create = Sensoren anlegen

sensors-mode-missing = Bitte den Modus (automatisch, Wert oder Zähler) wählen.
sensors-scale-not-number = Der Faktor muss eine Zahl sein.
sensors-type-missing = Bitte einen Sensortyp wählen.
sensors-name-missing = Bitte einen Namen angeben (die OID ist in keiner geladenen MIB bekannt).
form-too-large = Das Formular ist zu groß.
sensors-select-interface = Bitte mindestens ein Interface auswählen.
sensors-select-sensor = Bitte mindestens einen Sensor auswählen.

detail-last-poll = letzte Abfrage
detail-range = Zeitraum
detail-latest = Letzte Werte
detail-no-value = noch kein Messwert
detail-no-samples = Noch keine Messwerte im gewählten Zeitraum.
detail-chart-needs-js = Für den Graphen wird JavaScript benötigt.
detail-samples = Messwerte
# Wie weit die Tabelle der Messwerte zurückreicht ($minutes: 15, 30, 60, 360, 1440 oder 10080).
detail-span = { $minutes ->
    [60] letzte Stunde
    [360] letzte 6 Stunden
    [1440] letzte 24 Stunden
    [10080] letzte 7 Tage
   *[other] letzte { $minutes } Minuten
}
detail-table-needs-js = Für die Tabelle wird JavaScript benötigt.
detail-new-name = Neuer Name
detail-rename = Umbenennen
detail-rename-hint = Höchstens 64 Zeichen, je Gerät eindeutig. Messwerte, Graph, Schwellwerte und Status bleiben erhalten.
detail-interval = Abfrageintervall
detail-interval-seconds = Intervall in Sekunden
detail-interval-own = eigenes Intervall
detail-interval-hint = Leer lassen für das Intervall des Geräts ({ $seconds } s). 10 bis 3600 Sekunden; kürzere Intervalle ergeben mehr Messwerte und eine größere Datenbank.
detail-interval-invalid = Bitte eine ganze Zahl von Sekunden eingeben oder das Feld leer lassen.
detail-states = Statuswerte
detail-states-rated = bewertet
detail-states-stored = Jeder Wert bestimmt den Sensorstatus. Werte, die hier nicht stehen, gelten als Warnung.
detail-states-interface = Standard: nur „up“ ist in Ordnung, alles andere ist ein Ausfall. Hier lässt sich das je Wert anpassen, z. B. „down“ nur als Warnung.
detail-states-unrated = Noch nicht bewertet – der Wert ändert den Sensorstatus nicht. Unten steht ein Vorschlag aus den Namen der Werte; erst „Speichern“ schaltet die Bewertung ein.
detail-value = Wert
detail-rating = Bewertung
detail-rating-for = Bewertung für { $value }
detail-states-restore = Standard wiederherstellen
detail-states-off = Bewertung ausschalten
detail-thresholds = Schwellwerte
detail-thresholds-hint = Ein Wert löst aus, wenn er die Grenze überschreitet („über“) oder unterschreitet („unter“). „Ausfall“ und „Warnung“ gelten sofort. Leere Felder bedeuten: keine Grenze.
detail-in-unit = (in { $unit })
threshold-warn-above = Warnung über
threshold-warn-below = Warnung unter
threshold-down-above = Ausfall über
threshold-down-below = Ausfall unter
detail-thresholds-save = Schwellwerte speichern
detail-thresholds-admins = Die Schwellwerte können nur Administratoren ändern.
detail-events = Letzte Statuswechsel
detail-no-events = Noch keine Ereignisse.
detail-when = Zeitpunkt
detail-change = Wechsel

sensor-name-taken = Ein Sensor mit diesem Namen existiert an diesem Gerät bereits.
sensor-device-missing = Das gewählte Gerät existiert nicht.
states-rating-invalid = Ungültige Bewertung.
states-label-invalid = Ungültiger Name eines Statuswerts.
sensors-current = Aktuell
sensor-delete = Sensor löschen
sensor-delete-hint = Entfernt den Sensor mit allen Messwerten und seinem Verlauf. Das lässt sich nicht rückgängig machen.
sensor-delete-confirm = Ja, diesen Sensor löschen
sensors-trend-of = Verlauf der letzten Stunde: { $name }
