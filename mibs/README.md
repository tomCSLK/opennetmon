# Mitgelieferte Standard-MIBs

Diese MIB-Module werden in das netmon-Binary eingebettet und stehen immer zur Verfügung. Herstellerspezifische MIBs
(z. B. von TP-Link) werden in der Oberfläche unter „MIB-Dateien“ hochgeladen und gehören nicht hierher.

| Datei | Quelle |
|---|---|
| SNMPv2-SMI, SNMPv2-TC, SNMPv2-CONF, SNMPv2-MIB, RFC1155-SMI, RFC-1215, RFC1213-MIB, IF-MIB, IANAifType-MIB, SNMP-FRAMEWORK-MIB, HOST-RESOURCES-MIB, INET-ADDRESS-MIB | net-snmp 5.9.4, Verzeichnis `mibs/` (https://github.com/net-snmp/net-snmp/tree/v5.9.4/mibs), unverändert |
| ENTITY-MIB | aus RFC 4133 (https://www.rfc-editor.org/rfc/rfc4133.txt), Modultext ohne Seitenköpfe/-füße extrahiert |
| ENTITY-SENSOR-MIB | aus RFC 3433 (https://www.rfc-editor.org/rfc/rfc3433.txt), Modultext ohne Seitenköpfe/-füße extrahiert |

Die Module stammen aus IETF-RFCs (bzw. von der IANA gepflegt) und dürfen laut den Urheberrechtshinweisen der RFCs für Implementierungen
kopiert und weitergegeben werden; net-snmp liefert sie in derselben Form aus.

`RFC-1212` fehlt bewusst: Es definiert nur das Makro `OBJECT-TYPE` für SMIv1, das der Parser eingebaut kennt.
