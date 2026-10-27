# E-mail notification

When the status of a sensor changes, opennetmon sends an e-mail. The settings are **in the user interface** under **E-mail** (administrators only,
`/settings/mail`) and take effect immediately – no restart is needed.

## Settings

| Field | Meaning |
|---|---|
| Notifications on | Main switch. Off: nothing is sent (changes are still logged). |
| Also notify on warnings | Off: only failures and the recovery after a failure are reported. |
| Server, port | SMTP server (host name or IP). Usual: STARTTLS 587, TLS (SMTPS) 465, unencrypted 25. |
| Encryption | STARTTLS, TLS (SMTPS) or none (only for a trusted local relay). |
| User name, password | Optional. An empty user name = no login. The password is **never shown**; leaving it empty keeps the stored one. It is stored like the SNMP secrets with XChaCha20-Poly1305 (context `smtp.password`). |
| Sender, recipients | One sender address; one to 20 recipients (one address per line). Plain addresses only, no display names. |
| Language of the e-mails | German or English, **one language for all recipients** (alert and test e-mails; column `mail_settings.language`, migration `0014`, default German). Independent of the language in which the page is shown. |

With notifications switched off, the settings may be saved incomplete; switched on, they need a server, a sender and at least one recipient.
Invalid input is explained on the page, and the input stays (the password never).

**Send test e-mail** sends a test message with the *stored* settings (save first, then test) – even if notifications are still off.
The result appears immediately; errors (server not reachable, login rejected, recipient rejected, timeout) are shown in plain text.
The table “Recent deliveries” lists the last 20 attempts (alert or test, result, error text); entries are deleted after `retention_hourly_days`.

## What is reported?

| Change | Mail |
|---|---|
| any → `Down` (also on the very first value of a sensor) | yes |
| any → `Warning` | yes, if “Also notify on warnings” is on |
| `Down` → `OK` | yes (“OK again”) |
| `Warning` → `OK` | yes, if “Also notify on warnings” is on |
| first value `OK` (new sensor, server started) | no |
| → `Unknown` / `Paused` | no |
| `Paused` → any (first poll after resuming, without the old state) | only → `Down` or `Warning`, as for the first value |

Poll errors are only a warning at first, and only the second in a row is a failure (see `docs/poller.md`); threshold violations take effect immediately.

## Grouping and delivery (`src/alerting.rs`)

- The poller puts every status change into a queue (1000 slots, never blocking; if it is full, a warning is logged and the alert is dropped).
- A task of its own collects the changes arriving within **10 seconds** of the first one into **one mail** (“3 status changes: 2 down, 1 warning”).
- Changes of sensors that are paused through a **dependency** by then are left out; the outage of the sensor they depend on names what it paused
  (“Paused by it: 14 sensors on 3 devices”, see `docs/devices-and-groups.md`, “Dependencies”).
  If a whole device fails, this results in one mail instead of one per sensor. A single change has a subject of its own, e.g.
  `[opennetmon] Down: Switch Office / Ping`.
- The settings are read fresh for every mail. When the server shuts down, a collection that is still waiting is sent.
- Subject and text come from the catalogs (`alert-…` in `locales/<code>/mail.ftl`) in the configured language, e.g. `[opennetmon] Ausfall: Switch Büro / Ping`
  in German or “4 status changes: 1 down, 2 warnings, 1 OK again” in English. The body is plain text (time in UTC, device / sensor, old → new, message).
  There is **no automatic retry** on delivery errors; the error is in the log and in the table.

## Limits

- TLS certificates are verified against the public certificate authorities (Mozilla list, `webpki-roots`). A mail server with a private or self-signed
  certificate therefore does not work over STARTTLS/TLS; options would be your own CA certificate or “do not verify the certificate” (not implemented yet),
  or a local relay without encryption.
- No links to the web interface in the mail (the server does not know its public address).
- No escalation, no quiet hours, no notifications per device or group.

## Development and tests

- `dev/smtp_sink.py [port]` is a throwaway SMTP server (standard library, port 1025) that stores all mails as `.eml` under `dev/data/mail/`.
  In opennetmon: server `127.0.0.1`, port `1025`, encryption “No encryption”, no login. Never expose it to the network.
- `src/mail/testsmtp.rs` is a small test SMTP server for the Rust tests (optionally with `AUTH PLAIN`, optionally rejecting recipients).
- Tested: validation of addresses and settings, delivery with and without login, error cases, encrypted storage of the password, selection of the changes worth
  reporting, grouping, shutdown with a waiting mail, the chain “device fails → one mail”, the mail language (subject, plurals, an English test mail with a German
  user interface) and the web interface (permissions, CSRF, password never in the HTML, test mail).
