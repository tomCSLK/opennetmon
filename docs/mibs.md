# MIB files

opennetmon understands MIB files. This lets you create custom OID sensors **by name** (`tpSysMonitorCpu5Seconds.1` instead of
`1.3.6.1.4.1.11863.6.4.1.1.1.1.2.1`), and evaluation, unit and plain text for status values come from the MIB.

## Bundled standard MIBs

The binary contains the common standard MIBs (sources and license in `mibs/README.md`): SNMPv2-SMI/-TC/-CONF/-MIB, RFC1155-SMI, RFC-1215,
RFC1213-MIB (MIB-II), IF-MIB, IANAifType-MIB, SNMP-FRAMEWORK-MIB, HOST-RESOURCES-MIB (CPU, memory, disks of many devices),
INET-ADDRESS-MIB, ENTITY-MIB and ENTITY-SENSOR-MIB (temperatures and the like). They are always loaded and cannot be replaced by uploads.

## Uploading vendor MIBs (`/mibs`, administrators only)

- Under **MIB files**, choose one or many files at once (e.g. the vendor's whole unpacked folder) and upload them.
  For the TP-Link switches this is the package “TP-Link_L2 Switch_MIB” (`L2-tplinkMibs.zip`, 104 files) from the model's Omada support page.
- Every file is checked: text with at least one module (“… DEFINITIONS ::= BEGIN”), at most 2 MB per file, 300 files and 20 MB per upload.
  Files that only contain a bundled module (e.g. TP-Link's `RFC1213-MIB.mib`) are skipped.
- If a new file contains a module that is already loaded, it **replaces** the older version.
- The list shows the modules of each file with the number of objects and any problems: **missing: X-MIB** (an imported file is still missing – upload it),
  **n definitions cannot be resolved** (e.g. a reference to a node defined nowhere – as for 19 product identifiers in TP-Link's `tplink-products.mib`,
  irrelevant for monitoring).
- Deleting removes the file; the index is rebuilt immediately. Existing sensors stay unchanged (they store the numeric OID); only names and plain text in the
  display disappear.
- **Search** on the same page: by name or by a term in the description; hits with OID, kind (scalar, table column, table, …), type, unit, access and
  description. Pollable values come first and show the entry for the OID field.

## Custom OID by name

In the form “Add sensor” (type “Custom OID”; typing into the OID field selects the type automatically):

| Input | Meaning |
|---|---|
| `tpSysMonitorCpu5Seconds.1` | name plus index (table column; index = row, here unit 1) |
| `sysUpTime` | scalar; `.0` is appended automatically |
| `IF-MIB::ifInErrors.3` | with module, if a name occurs in several MIBs with different OIDs |
| `1.3.6.1.2.1.1.3.0` | still numeric; if the OID is known, it is named anyway |

- Suggestions appear while typing; for a known name the line below shows module, kind, type, unit, OID and description
  (for table columns also which index to append).
- **Evaluation “Automatic”**: counters (`Counter32/64`) are measured as a rate per second, everything else as a value. Value/counter can still be chosen
  explicitly.
- **Unit**: empty = `UNITS` from the MIB (if present); otherwise enter it freely (e.g. `%`, at most 24 characters). It is shown after every value.
- **Name**: empty = object name with index (e.g. “tpSysMonitorCpu5Seconds.1”).
- Clear error messages for tables/rows/nodes (“not a pollable value”), columns without an index, objects that cannot be read and unknown names.

## Find values: taking over tables as sensors (`/devices/<id>/values`, administrators only)

On a device's sensor page, **Find values** opens a list of all values the device returns in known tables:

| Table | Contents | preselected |
|---|---|---|
| `tpPoePortConfigTable` (TP-Link) | per port PoE power (W), status, current (mA), voltage (V), class | power, status |
| `tpPoeGlobal` (TP-Link) | PoE consumption total, remaining, budget (W) | consumption |
| `tpSysMonitorCpuTable`, `tpSysMonitorMemoryTable` (TP-Link) | CPU (5 s, 1 min, 5 min) and memory in % | CPU 1 min, memory |
| `hrProcessorTable`, `hrStorageTable` (HOST-RESOURCES-MIB) | CPU load per core; usage and size per storage (MB) | load, used |
| `entPhySensorTable` (ENTITY-SENSOR-MIB) | temperatures, fans, voltages, currents with names from `entPhysicalName` | reading |

- Tables whose MIB is not loaded or which the device does not answer are simply missing (the TP-Link entries need the uploaded TP-Link MIBs).
- **Another table or group**: enter the name at the top (e.g. `tpPoePortConfigTable`, `IF-MIB::ifXTable` or a group of scalars).
  Then all numeric columns are read; only pure measurements (read-only) are preselected, no settings.
- **Names** of the rows: ports via `ifName`/`ifDescr`, hardware via `entPhysicalName`, otherwise a text column of the table (…Name, …Descr, …Label), otherwise
  “Index N”. Every name can be changed before creating.
- **Unit and factor**: from `UNITS`, otherwise from the description (TP-Link writes e.g. “in 0.1W” → factor 0.1, unit W), for hardware sensors from type,
  scale and precision of the ENTITY-SENSOR-MIB. Counters are created as a rate.
- **New sensors are paused**: they are not polled and trigger no notification until an administrator resumes them.
  To poll them right away, tick “Start now”. OIDs that are already monitored are locked as “exists”; equal names are numbered.
- **Resume**: on the sensor page, paused sensors appear in a collapsible section of their own, “Paused sensors (n)”, with “Resume selected”, “Resume all” and
  “Delete selected” (`POST /devices/<id>/sensors/bulk`). Running sensors can be paused again one by one (`docs/devices-and-groups.md`, “Pausing”). In the
  overview, paused sensors appear as an outlined label of their own and do not affect the status.
- Limits: timeout at most 10 s, one retry; at most 3000 rows per column and 500 values per search.
- Measured on the SG2008P: 4 PoE ports (port 1: 6.3 W, 119 mA, 52.9 V, class 4), total consumption 6.5 W with a 62 W budget, CPU 5 %, memory 53 %.
  The SG2008 return CPU and memory, but neither HOST-RESOURCES nor ENTITY tables.

## Display

- The sensor list and the detail page name OID sensors with name and module (“tpSysMonitorCpu5Seconds.1 (TPLINK-SYSMONITOR-MIB) · value, unit %”).
- **Status values in plain text**: if the object has an enumeration (e.g. `ifOperStatus`: 1 = up, 2 = down …), tile, chart axis and table show “down (2)”.
  This also applies to the built-in interface status sensor.
- **Rating status values:** for each value of an enumeration you can define whether it is OK, a warning or down; new sensors get a suggestion from the names
  of the values (`docs/charts-and-thresholds.md`, section “Rating status values”).

## Technical details

- `src/mib/lexer.rs`, `src/mib/parser.rs`: a tolerant parser for SMIv1/SMIv2 (imports, OID assignments, OBJECT-TYPE with SYNTAX/UNITS/ACCESS/
  DESCRIPTION/INDEX/AUGMENTS, textual conventions, enumerations; macro definitions, compliance sections and the like are skipped).
  Vendor quirks such as `n/a(0)` or `power_user(1)` in enumerations and Windows line endings are handled.
- `src/mib/mod.rs`: resolution across module boundaries (imports, if necessary a unique name in another module), base types via textual conventions,
  tables/rows/columns including `AUGMENTS`, search, name ↔ OID. The index lives in memory and is rebuilt at startup and after every upload/deletion
  (database: only the file texts, table `mib_files`, migration `0009`).
- Cross-check with net-snmp: `dev/mib-oracle.sh dev/data/mibs/tplink/*.mib` compares all resolved OIDs with `snmptranslate`.
  Result with the standard and TP-Link MIBs: 2963 common names, **0 differences**; opennetmon knows 230 more objects because net-snmp rejects two faulty
  TP-Link files.
- `src/mib/catalog.rs`: known tables with columns, names, factors and units; units from descriptions (`unit_from_description`).
  `src/value_discovery.rs`: walk per column, row names, suggestions; `src/web/sensor_values.rs`: search page and creation.
  A walk that hits a non-increasing OID (faulty agents) ends there instead of aborting the whole search.
- Tests: lexer/parser (including truncated and broken input), all bundled MIBs without errors, known OIDs/types/enumerations, name resolution,
  upload as multipart, replacing, deleting, permissions, sensors by name; the TP-Link files are tested too if they are present locally under
  `dev/data/mibs/tplink`.
