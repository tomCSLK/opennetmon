template-named-error = „{ $name }“: { $message }
template-channel-invalid = ungültiger Kanal.
template-states-invalid = ungültige Statusbewertung.
template-version-unknown = Unbekannte Vorlagenversion { $version }.
template-too-many = Eine Vorlage enthält höchstens { $count } Einträge.
template-interface-invalid = ungültiger Interface-Name.
template-custom-values =
    { $count ->
        [one] { $count } eigener Wert
       *[other] { $count } eigene Werte
    }
template-services =
    { $count ->
        [one] { $count } Dienst
       *[other] { $count } Dienste
    }
template-empty = leer
template-ports-same =
    { $count ->
        [one] { $count } bestimmter Port
       *[other] { $count } bestimmte Ports
    }
template-ports-up = aktive Ports
template-ports-all = alle Ports
template-interfaces-unread = Die Interfaces des Geräts konnten nicht gelesen werden; „dieselben Ports“ ist deshalb nicht möglich.
template-would-be-empty = Die Vorlage wäre leer. Bitte mindestens einen Sensor übernehmen.
template-skipped-unread = { $what }: Die Interfaces wurden nicht gelesen.
template-skipped-interface = { $name }: Das Gerät hat kein Interface „{ $interface }“.
template-skipped-oid = { $name }: Das Gerät liefert für { $oid } keinen Wert.

template-with-thresholds-and-states = mit Schwellwerten und Statusbewertung
template-with-thresholds = mit Schwellwerten
template-with-states = mit Statusbewertung
template-rule-port = { $what } an „{ $interface }“
template-rule-each-port = je Port ein Sensor „{ $what } <Portname>“
template-oid-counter = Zähler
template-oid-value = Wert
template-traffic = Traffic
template-interface-status = Interface-Status
template-result-created =
    { $count ->
        [one] { $count } Sensor angelegt
       *[other] { $count } Sensoren angelegt
    }
template-result-present = , { $count } schon vorhanden
template-result-skipped = , { $count } übersprungen
template-no-match = Keine Vorlage passt zur Gerätekennung { $oid }. Eine Vorlage lässt sich unter „Vorlage anwenden“ auch von Hand wählen.
template-no-object-id = Das Gerät meldet keine Gerätekennung, daher passt keine Vorlage. Eine Vorlage lässt sich unter „Vorlage anwenden“ auch von Hand wählen.
template-choose = Bitte eine Vorlage wählen.
template-applied-to = Vorlage angewendet auf { $device }
template-option-none = Keine – nur das Gerät anlegen
template-option-auto = Automatisch anhand der Gerätekennung erkennen

templates-subtitle = Sensoren eines fertig eingerichteten Geräts auf weitere Geräte übertragen
templates-none = Noch keine Vorlagen.
templates-none-hint = Richte ein Gerät fertig ein und wähle auf seiner Sensorseite unter „Gerätevorlage“ den Knopf „Als Vorlage speichern“.
template = Vorlage
template-contents = Inhalt
template-object-id = Gerätekennung
template-from = aus { $device }
template-by-hand-only = – (nur von Hand)
templates-how = So funktionieren Vorlagen
templates-how-save = „Als Vorlage speichern“ auf der Sensorseite eines Geräts übernimmt Ping, Uptime, Interface-Sensoren und eigene Werte (z. B. PoE, CPU) samt Namen, Zustand (aktiv/pausiert), Faktor, Einheit und Schwellwerten.
templates-how-apply = Beim Anlegen eines Geräts oder später mit „Vorlage anwenden“ werden nur fehlende Sensoren angelegt; Werte, die das Gerät nicht liefert, werden übersprungen.
templates-how-detect = Die Gerätekennung (sysObjectID) bestimmt, welche Vorlage automatisch vorgeschlagen wird.
template-created-from = erstellt aus { $device }
count-entries =
    { $count ->
        [one] { $count } Eintrag
       *[other] { $count } Einträge
    }
template-rule = Regel
template-change-hint = Den Inhalt änderst du, indem du ein Gerät passend einrichtest und daraus eine neue Vorlage speicherst.
template-name-detection = Name und Erkennung
template-object-id-label = Gerätekennung (sysObjectID)
template-object-id-placeholder = leer: nur von Hand wählbar
template-detection-hint = Geräte mit genau dieser Kennung bekommen die Vorlage beim Anlegen („automatisch erkennen“) und unter „Vorlage anwenden“ vorgeschlagen.
template-delete-hint = Geräte, die mit dieser Vorlage eingerichtet wurden, behalten ihre Sensoren.
template-delete = Vorlage löschen
template-from-device = Vorlage aus { $device }
template-new-subtitle = Was übernommen wird, legt opennetmon später auf anderen Geräten an.
template-read-partly = Das Gerät konnte nicht vollständig gelesen werden: { $message } Gerätekennung und „dieselben Ports“ fehlen deshalb.
template-name = Name der Vorlage
template-new-detection-hint = Geräte mit derselben Kennung (gleiches Modell) bekommen diese Vorlage automatisch vorgeschlagen.
template-no-sensors-of-kind = Dieses Gerät hat keine Sensoren vom Typ { $kind }.
template-on-device = Am Gerät:
template-choice-up = alle aktiven Ports des Zielgeräts
template-choice-all = alle Ports, auch inaktive
template-choice-same = dieselben Ports (nach Portnamen, mit den Sensornamen von hier)
template-choice-skip = nicht übernehmen
template-other-sensors = Weitere Sensoren
template-no-other-sensors = Keine Ping-, Uptime- oder eigenen Sensoren.
template-take = Übernehmen
template-take-named = { $name } übernehmen
template-save = Vorlage speichern
template-new-hint = Übernommen werden Namen, Zustand (aktiv/pausiert), Faktor, Einheit und Schwellwerte. Eigene Werte (z. B. PoE je Port) werden mit derselben OID angelegt; liefert ein Zielgerät dafür keinen Wert, wird der Sensor übersprungen.
template-identification = Kennung
template-object-id-unread = Die Gerätekennung konnte nicht gelesen werden: { $message }
templates-none-apply = Speichere zuerst ein fertig eingerichtetes Gerät als Vorlage.
template-detected = Passend zur Gerätekennung: { $name }.
template-apply-hint = Angelegt werden nur Sensoren, die noch fehlen. Vorhandene Sensoren bleiben unverändert, nichts wird gelöscht.
template-apply-button = Anwenden
template-named = Vorlage „{ $name }“
template-not-applied = Die Vorlage wurde nicht angewendet. { $message }
template-result-created-title = Angelegt
template-result-skipped-title = Übersprungen
to-sensors = Zu den Sensoren
to-overview = Zur Übersicht

template-object-id-invalid = Die Gerätekennung muss eine numerische OID sein, z. B. 1.3.6.1.4.1.11863.1.1.11.
template-name-taken = Eine Vorlage mit diesem Namen gibt es bereits.
