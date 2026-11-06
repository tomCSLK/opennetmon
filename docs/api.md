# Read-only API

opennetmon has a small read-only API for other programs. Today it has one endpoint: the **inventory of devices**.
[opensecmon](https://github.com/tomCSLK/opensecmon) reads it to suggest which software and hardware to watch for security advisories.
Nothing can be changed through the API.

## Access tokens

Programs identify themselves with a **token**, not with a user name and password. Administrators manage tokens under
**Settings → API access** (`/api-tokens`):

- **Create:** give the token a name that says what it is for (1–64 characters, unique). The token is shown **once**, in the reply to the form.
  Copy it into the other program; it cannot be looked at later.
- **Form of a token:** `onm_` and 64 hexadecimal characters (256 random bits). The prefix makes a leaked token easy to recognise.
- **Stored:** only the SHA-256 of the token (like sessions), so a copy of the database or a backup does not give access.
  The page lists name, who created the token, when, and when it was last used.
- **Revoke:** takes effect at once. If a token is lost, revoke it and create a new one.
- **Audit log:** creating and revoking a token appears in the audit log (`/audit`, kind “API token”), never the token itself.

## `GET /api/v1/inventory`

```
curl -H "Authorization: Bearer onm_…" https://opennetmon.example/api/v1/inventory
```

Answers `200` with `Content-Type: application/json`:

```json
{
  "devices": [
    { "id": 17, "name": "Router", "address": "192.0.2.1",
      "sys_descr": "Omada ER605 …", "sys_object_id": "1.3.6.1.4.1.11863.5" }
  ]
}
```

| Field | Meaning |
|---|---|
| `id` | The stable ID of the device in opennetmon (it does not change when the device is renamed). |
| `name` | The name of the device. |
| `address` | Host name or IP address as entered. |
| `sys_descr` | `sysDescr` as the device last reported it (at most 1024 characters), empty if unknown. |
| `sys_object_id` | `sysObjectID` in dotted form, empty if unknown. |

All devices are listed, **paused ones too**, ordered by `id`. Devices without SNMP, and devices that never answered, have empty
`sys_descr` and `sys_object_id`. The values are text sent by the devices: treat them as data.

### Where `sys_descr` and `sys_object_id` come from

- The poller asks each device with an SNMP session in the cycle (any SNMP sensor) for both values at its first cycle and then about **once a day**
  (one hour later after a failure). Devices with only ping or service sensors are not asked.
- The **connection test** on the device page stores them too.

### Errors

| Status | When |
|---|---|
| `401` | No token, a wrong or revoked token, a wrong scheme. `WWW-Authenticate: Bearer`. A browser session does not count as a token. |
| `429` | Too many failed attempts from this address, `Retry-After` in seconds. |

Failed attempts are counted per client address like failed sign-ins (a missing or wrong token counts as a failure, a good one clears the count):
five failures refuse further attempts for 5 s, doubling up to 60 s, and ten failures from one address for 30 s up to 15 min. While an address is
refused, even a correct token gets `429`. Behind a reverse proxy set `client_ip_header` ([`users-and-login.md`](users-and-login.md)),
otherwise all requests share the address of the proxy.

### Transport

Use HTTPS (the built-in HTTPS or a reverse proxy): the token travels in a header. Plain HTTP only makes sense on the same machine (`127.0.0.1`).
