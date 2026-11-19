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
| Address of the web interface | Optional, e.g. `https://opennetmon.example.org`. With it, the mails get buttons and links into the web interface (sensor, device, log, report, settings); empty = no links at all. Must be an absolute `http://` or `https://` address without user name/password, query (`?`) or fragment (`#`), at most 200 characters; a path is allowed (`https://example.org/opennetmon`), a trailing slash is dropped. Column `mail_settings.ui_url`, migration `0026`; changes are recorded in the change log. |

With notifications switched off, the settings may be saved incomplete; switched on, they need a server, a sender and at least one recipient.
Invalid input is explained on the page, and the input stays (the password never).

**Send test e-mail** sends a test message with the *stored* settings (save first, then test) – even if notifications are still off.
It is formatted like the notifications and lists server and port, encryption, whether a login is used (never the user name or the password), the recipients,
which changes are reported and the language.
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
  `[opennetmon] Down: Switch Office · Ping – No response`.
- The settings are read fresh for every mail. When the server shuts down, a collection that is still waiting is sent.
- Subject and text come from the catalogs (`alert-…` and `mail-…` in `locales/<code>/mail.ftl`) in the configured language.
  There is **no automatic retry** on delivery errors; the error is in the log and in the table.

## What a mail looks like (`src/alerting/compose.rs`, `templates/mail/`)

Every notification and the test mail are sent as **`multipart/alternative`**: a plain-text part and an HTML part. The HTML part is
`multipart/related` with the logo as an inline image (`cid:`); nothing is loaded from the internet, so mail programs that block remote images show
the mail completely. The HTML is built from tables with the styles on the elements (as mail programs need it); a small `<style>` block only adds
the dark mode and the layout for phones. Device, sensor and group names and the messages are escaped.

- **Subject:** `[opennetmon] Down: Switch Office · Ping – No response`, for a recovery `[opennetmon] OK again: Switch Office · Ping (down for 42 min)`,
  for several changes `[opennetmon] 4 status changes: 1 down, 2 warnings, 1 OK again`. Line breaks and control characters are removed from
  everything that goes into a header.
- **Times** are local times of the server's time zone (the one of the maintenance windows, `time_zone` in the config, otherwise the system's);
  the zone is named in the footer and in the plain text, e.g. `Thu, 11/19/2026 · 14:32 (Europe/Berlin)`.
- **A single change:** coloured band and status chip, device · sensor with group path and address, the message, and key figures as far as they
  are known (each is left out if it cannot be read):
  - *outage:* since when (and how long the status before lasted), the last value with its time, the limits, availability and outages of the last 7 days;
  - *warning:* the current value (since when beyond which limit), the trend of the last 2 hours with the range before it, the limits and the highest
    value of the 7 days before the warning (the lowest for a limit below); then up to 5 other sensors of the device with value and status;
  - *recovery:* how long the outage lasted, the current value with the 6-hour average, outages and availability of the last 7 days; if the
    sensor only says “OK”, the message box shows the value instead (“Answers again – 1.4 ms” for ping and service sensors). 
  Below them the **last 6 hours as a picture**: a line chart of the raw values of the main channel, drawn by the server as a PNG
  (1120 × 392 pixels, shown 560 wide, on a white card also in dark mode) and attached as a second inline image (`cid:chart@opennetmon`),
  with the limits as dashed lines, the outages tinted and labelled (“no answer” while it lasts, its length after a recovery), temperatures
  in orange. It has an alt text (“Response time over the last 6 hours, no answer since 14:32”). If there are no values in the window or
  the picture cannot be drawn, the mail shows the history as **bars** instead (bucket averages; values beyond a limit in the colour of
  the limit, outages tinted), with a warning in the log; sensors whose value is a status always get coloured columns. An outage lists the devices it **pauses through
  dependencies** (with the number of sensors), a recovery says which paused sensors run again. Numbers and their units never wrap apart. Buttons: open sensor, open device and, depending on the
  change, the sensor's log, the limits or the 7-day report.
- **Several changes (one mail within 10 s):** counters per status, the **probable cause** (the outage that paused the most through dependencies, if
  any), then the changes worst first with status, message, time and, for the first 10, a small history. At most 30 changes are listed, the rest is
  counted (“… and 12 more”), so the mail stays well below the size at which Gmail cuts mails off (about 100 KB). Buttons: overview and log.
- **Links** only exist when the **address of the web interface** is set (see Settings). They are built from that address plus fixed paths and
  numeric ids, nothing else.
- Reading the extra figures costs a few small queries per mail. If one of them fails, the mail goes out without that figure (with a warning in the log);
  a mail is never held back for it.

## Limits

- TLS certificates are verified against the public certificate authorities (Mozilla list, `webpki-roots`). A mail server with a private or self-signed
  certificate therefore does not work over STARTTLS/TLS; options would be your own CA certificate or “do not verify the certificate” (not implemented yet),
  or a local relay without encryption.
- Links in the mails need the address of the web interface to be set by hand (the server does not know its public address).
- The summary mail has small bar histories, no pictures. No weekly report.
- No escalation, no quiet hours, no notifications per device or group.

## Development and tests

- `dev/smtp_sink.py [port]` is a throwaway SMTP server (standard library, port 1025) that stores all mails as `.eml` under `dev/data/mail/`.
  In opennetmon: server `127.0.0.1`, port `1025`, encryption “No encryption”, no login. Never expose it to the network.
- `src/mail/testsmtp.rs` is a small test SMTP server for the Rust tests (optionally with `AUTH PLAIN`, optionally rejecting recipients).
- Tested: validation of addresses and settings, delivery with and without login, error cases, encrypted storage of the password, selection of the changes worth
  reporting, grouping, shutdown with a waiting mail, the chain “device fails → one mail”, the mail language (subject, plurals, an English test mail with a German
  user interface) and the web interface (permissions, CSRF, password never in the HTML, test mail). The composed mails are tested for content in both languages, with and
  without the address of the web interface, escaping, single-line headers, no external sources, the cap of the summary mail and its size; a test with the test SMTP
  server checks the MIME structure (text, HTML, inline logo).
- `cargo test --lib mail_preview -- --ignored` writes sample mails (outage, recovery, warning, summary, test mail; German) to `target/mail-preview/*.html`
  and `*.txt` to look at in a browser (there the logo is embedded as a `data:` URI; the real mails use `cid:`).
