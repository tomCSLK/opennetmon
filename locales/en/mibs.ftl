mib-type-octet-string = Text/bytes
mib-type-ip-address = IP address
mib-kind-node = Node
mib-kind-scalar = Scalar
mib-kind-table = Table
mib-kind-row = Table row
mib-kind-column = Table column
mib-instance-not-number = “{ $part }” is not a number in the instance part of the OID.
mib-name-unknown = “{ $name }” is not known in any loaded MIB.
mib-name-ambiguous = “{ $name }” is ambiguous ({ $modules }). Please add the module, e.g. { $module }::{ $name }.
mib-oid-missing = Please enter an OID or a MIB name.
mib-not-pollable = “{ $name }” is of kind { $kind }, not a pollable value. Please choose a scalar or a table column.
mib-not-readable = “{ $name }” is not readable according to its MIB ({ $access }).
mib-column-needs-index = “{ $name }” is a table column: please append the index, e.g. { $name }.1 (index: { $index }).
mib-column-needs-index-unknown = “{ $name }” is a table column: please append the index, e.g. { $name }.1 (index unknown).

mibs-uploaded =
    { $files ->
        [one] { $files } file
       *[other] { $files } files
    } with { $modules ->
        [one] { $modules } module
       *[other] { $modules } modules
    } imported
mibs-replaced =
    { $count ->
        [one] , { $count } older version replaced
       *[other] , { $count } older versions replaced
    }
mibs-shadowed = skipped: comes with opennetmon
mibs-missing = missing: { $modules }
mibs-unresolved =
    { $count ->
        [one] { $count } definition not resolvable
       *[other] { $count } definitions not resolvable
    }
mibs-upload-broken = The upload is incomplete or too large: { $detail }
mibs-file-unreadable = The file { $name } could not be read: { $detail }
mibs-too-many-files = At most { $count } files at once.
mibs-skipped-too-large = { $name }: larger than 2 MB
mibs-skipped-no-mib = { $name }: no MIB definition (“… DEFINITIONS ::= BEGIN”) found
mibs-skipped-bundled = { $name }: { $modules } comes with opennetmon
mibs-select-file = Please select at least one file.
mibs-subtitle = { $modules } modules with { $objects } objects loaded. Uploaded vendor MIBs provide names, types, units and plain text for status values of custom OID sensors.
mibs-skipped = { $count } skipped:
mibs-upload = Upload
mibs-choose-files = Choose MIB files
mibs-upload-hint = Choose several files at once (e.g. a vendor's whole unpacked folder). A file with a module that is already loaded replaces the older version. Standard MIBs such as IF-MIB or HOST-RESOURCES-MIB are included.
mibs-search = Search
mibs-search-placeholder = Name or term, e.g. tpSysMonitorCpu, temperature, ifHCInOctets
mibs-search-label = Search MIB objects
mibs-nothing-found = Nothing found.
mibs-kind-type = Kind / type
mibs-uploaded-files = Uploaded files
mibs-none-uploaded = No vendor MIBs uploaded yet.
mibs-file = File
mibs-modules = Modules
mibs-uploaded-at = Uploaded
mibs-files-summary =
    { $files ->
        [one] 1 file
       *[other] { $files } files
    }{ $problems ->
        [0] , all fine
        [one] , 1 with problems
       *[other] , { $problems } with problems
    }
mibs-files-count =
    { $count ->
        [one] 1 file
       *[other] { $count } files
    }
mibs-vendor-number = Enterprise { $number }
mibs-vendor-standard = Standard MIBs (IETF)
mibs-vendor-none = No vendor
mibs-files-show = Show files
mibs-files-all = All ({ $count })
mibs-files-problems = With problems ({ $count })
mibs-files-ok = Fine ({ $count })
mibs-files-filter = Filter by file, module or vendor
mibs-files-none-match = No file matches the selection.
count-objects =
    { $count ->
        [one] { $count } object
       *[other] { $count } objects
    }
mibs-bundled = Bundled standard MIBs ({ $count })
mibs-module = Module
mibs-objects = Objects
