mib-type-octet-string = Text/Bytes
mib-type-ip-address = IP-Adresse
mib-kind-node = Knoten
mib-kind-scalar = Einzelwert
mib-kind-table = Tabelle
mib-kind-row = Tabellenzeile
mib-kind-column = Tabellenspalte
mib-instance-not-number = „{ $part }“ ist keine Zahl im Instanzteil der OID.
mib-name-unknown = „{ $name }“ ist in keiner geladenen MIB bekannt.
mib-name-ambiguous = „{ $name }“ ist mehrdeutig ({ $modules }). Bitte mit Modul angeben, z. B. { $module }::{ $name }.
mib-oid-missing = Bitte eine OID oder einen MIB-Namen angeben.
mib-not-pollable = „{ $name }“ ist vom Typ { $kind }, kein abfragbarer Wert. Bitte einen Einzelwert oder eine Tabellenspalte wählen.
mib-not-readable = „{ $name }“ ist laut MIB nicht lesbar ({ $access }).
mib-column-needs-index = „{ $name }“ ist eine Tabellenspalte: bitte den Index anhängen, z. B. { $name }.1 (Index: { $index }).
mib-column-needs-index-unknown = „{ $name }“ ist eine Tabellenspalte: bitte den Index anhängen, z. B. { $name }.1 (Index unbekannt).

mibs-uploaded =
    { $files ->
        [one] { $files } Datei
       *[other] { $files } Dateien
    } mit { $modules ->
        [one] { $modules } Modul
       *[other] { $modules } Modulen
    } übernommen
mibs-replaced =
    { $count ->
        [one] , { $count } ältere Fassung ersetzt
       *[other] , { $count } ältere Fassungen ersetzt
    }
mibs-shadowed = übersprungen: wird mit opennetmon mitgeliefert
mibs-missing = fehlt: { $modules }
mibs-unresolved =
    { $count ->
        [one] { $count } Definition nicht auflösbar
       *[other] { $count } Definitionen nicht auflösbar
    }
mibs-upload-broken = Der Upload ist unvollständig oder zu groß: { $detail }
mibs-file-unreadable = Die Datei { $name } konnte nicht gelesen werden: { $detail }
mibs-too-many-files = Höchstens { $count } Dateien auf einmal.
mibs-skipped-too-large = { $name }: größer als 2 MB
mibs-skipped-no-mib = { $name }: keine MIB-Definition („… DEFINITIONS ::= BEGIN“) gefunden
mibs-skipped-bundled = { $name }: { $modules } wird mit opennetmon mitgeliefert
mibs-select-file = Bitte mindestens eine Datei auswählen.
mibs-subtitle = { $modules } Module mit { $objects } Objekten geladen. Hochgeladene Hersteller-MIBs liefern Namen, Typen, Einheiten und Klartext für Statuswerte bei eigenen OID-Sensoren.
mibs-skipped = { $count } übersprungen:
mibs-upload = Hochladen
mibs-choose-files = MIB-Dateien auswählen
mibs-upload-hint = Mehrere Dateien auf einmal auswählen (z. B. den ganzen entpackten Ordner eines Herstellers). Eine Datei, die ein bereits geladenes Modul enthält, ersetzt die ältere Fassung. Standard-MIBs wie IF-MIB oder HOST-RESOURCES-MIB sind schon dabei.
mibs-search = Suchen
mibs-search-placeholder = Name oder Begriff, z. B. tpSysMonitorCpu, temperature, ifHCInOctets
mibs-search-label = MIB-Objekt suchen
mibs-nothing-found = Nichts gefunden.
mibs-kind-type = Art / Typ
mibs-uploaded-files = Hochgeladene Dateien
mibs-none-uploaded = Noch keine Hersteller-MIBs hochgeladen.
mibs-file = Datei
mibs-modules = Module
mibs-uploaded-at = Hochgeladen
mibs-files-summary =
    { $files ->
        [one] 1 Datei
       *[other] { $files } Dateien
    }{ $problems ->
        [0] , alle in Ordnung
        [one] , 1 mit Problemen
       *[other] , { $problems } mit Problemen
    }
mibs-files-count =
    { $count ->
        [one] 1 Datei
       *[other] { $count } Dateien
    }
mibs-vendor-number = Unternehmen { $number }
mibs-vendor-standard = Standard-MIBs (IETF)
mibs-vendor-none = Ohne Hersteller
mibs-files-show = Dateien anzeigen
mibs-files-all = Alle ({ $count })
mibs-files-problems = Mit Problemen ({ $count })
mibs-files-ok = In Ordnung ({ $count })
mibs-files-filter = Datei, Modul oder Hersteller filtern
mibs-files-none-match = Keine Datei passt zur Auswahl.
count-objects =
    { $count ->
        [one] { $count } Objekt
       *[other] { $count } Objekte
    }
mibs-bundled = Mitgelieferte Standard-MIBs ({ $count })
mibs-module = Modul
mibs-objects = Objekte
