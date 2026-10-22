# Bundled standard MIBs

These MIB modules are embedded into the opennetmon binary and are always available. Vendor-specific MIBs (e.g. from TP-Link) are uploaded in the
user interface under “MIB files” and do not belong here.

| File | Source |
|---|---|
| SNMPv2-SMI, SNMPv2-TC, SNMPv2-CONF, SNMPv2-MIB, RFC1155-SMI, RFC-1215, RFC1213-MIB, IF-MIB, IANAifType-MIB, SNMP-FRAMEWORK-MIB, HOST-RESOURCES-MIB, INET-ADDRESS-MIB | net-snmp 5.9.4, directory `mibs/` (https://github.com/net-snmp/net-snmp/tree/v5.9.4/mibs), unchanged |
| ENTITY-MIB | from RFC 4133 (https://www.rfc-editor.org/rfc/rfc4133.txt), module text extracted without page headers/footers |
| ENTITY-SENSOR-MIB | from RFC 3433 (https://www.rfc-editor.org/rfc/rfc3433.txt), module text extracted without page headers/footers |

The modules come from IETF RFCs (or are maintained by IANA) and, according to the RFCs' copyright notices, may be copied and distributed for
implementations; net-snmp ships them in the same form.

`RFC-1212` is left out on purpose: it only defines the `OBJECT-TYPE` macro for SMIv1, which the parser knows built in.
