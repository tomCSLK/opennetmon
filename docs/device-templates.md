# Device templates

A device template transfers the sensors of a fully set-up device to other devices – for example from one switch to the other switches of the same
model. All template pages are for administrators only.

## Creating a template: “Save as template”

On the sensor page of a device, in the section **Device template** (`/devices/<id>/template/new`). opennetmon reads the device identification
(`sysObjectID`) and the interfaces from the device.

| Part | Options |
|---|---|
| Name | free, unique (suggestion: the name of the device) |
| Device identification | prefilled with the device's `sysObjectID`, e.g. `1.3.6.1.4.1.11863.5.134` (SG2008) or `…11863.5.154` (SG2008P); empty = can only be chosen by hand |
| Traffic, interface status | **all active ports** of the target device, **all ports** (inactive ones too), **the same ports** (by port name, with the sensor names of this device) or do not take over |
| Other sensors | ping, uptime and custom values (e.g. PoE, CPU, memory) individually by ticking them |

- Name, state (active/paused), factor, unit, thresholds and the rating of status values are taken over. With “active/all ports”, the thresholds only apply
  if all interface sensors of this kind on the source device have the same ones; the new sensors are active if at least one on the source device is active.
- “Ports” are interfaces of the types Ethernet and WLAN – the same selection as the preselection in “Find interfaces”
  (on TP-Link, `Vlan-interface1` counts as well).
- “The same ports” needs the interfaces of the source device; if it is not reachable, the option is locked.
- An empty template is rejected.

## Applying a template

- **When creating a device** (form “New device”, section “Device template”, as soon as templates exist):
  “Detect automatically from the device identification” (default), a specific template or none.
  After saving, a result page shows what was created and what was skipped.
- **Later** with “Apply template” on the sensor page (`/devices/<id>/template/apply`); the template matching the device identification is preselected,
  otherwise the one used last. This way, for example, newly used ports can be added.
- Only what is missing is created: ping/uptime if the device has no such sensor yet; interface sensors if there is none of this kind for the interface yet;
  custom values if the OID is not monitored yet. **Existing sensors are never changed or deleted.**
- Custom values are checked with a GET first; if the device returns no number (e.g. no PoE on the SG2008), the sensor is **skipped** and listed with the
  reason on the result page. The same applies to ports of “the same ports” that do not exist on the target device.
- Names already in use are numbered (“Uptime (2)”).
- If the device is not reachable or no template matches the identification, nothing is created; the device itself stays created.
- The device remembers the template applied last (“Set up with …” on the sensor page).
- Time limits as for the interface search: at most a 10 s timeout and 1 retry.

## Managing templates (`/templates`, menu item “Device templates”)

- A list with contents, device identification and the number of devices set up with it.
- Detail page: all rules; name and device identification can be changed; delete (devices keep their sensors).
- To change the contents, set up a device accordingly and save a new template from it.

## Measured on real hardware

Template from one switch (SG2008, ping, uptime, traffic of active ports) → a new device for a second SG2008 with “detect automatically”:
the template was found via `1.3.6.1.4.1.11863.5.134`, 7 sensors were created (ping, uptime, traffic on 5 active interfaces), in about 2 s.

## Technical details

- `src/templates.rs`: rules (`Body`, stored as JSON), `build` (from the sensors of a device), `plan` (pure logic: what is missing, what is skipped),
  `apply` (read interfaces, check OIDs with GETs in blocks of 10, create sensors with thresholds), `detect`.
- `src/storage/templates.rs`, migration `0011`: table `device_templates` (name, device identification, source device, rules) and `devices.template_id`
  (`ON DELETE SET NULL`). New dependency `serde_json` (was already in the build through axum).
- `src/web/templates.rs`, templates `templates_list.html`, `template_detail.html`, `template_new.html`, `template_apply.html`, `template_result.html`.
- Tests: rules and plan without a network, querying identification and values against the fake agent, the complete flow through the user interface
  (save template → create a device with detection → apply again), error cases and permissions.
