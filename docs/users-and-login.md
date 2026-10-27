# Users and login

## Roles

| Role | Permissions |
|---|---|
| Administrator (`admin`) | everything, including user management |
| Viewer (`viewer`) | read only |

Handlers that should only allow administrators call `CurrentUser::require_admin()`.

## Managing users (user interface, `/users`, administrators only)

Under **Settings → Users**:

- **List** with role, language, creation time and number of open sessions; your own account is marked “(you)”, users exempt from the
  automatic sign-out “stays signed in”.
- **New user:** user name, role (viewer or administrator), the password twice and the language (default “Same as the browser”).
  After an error, name, role and language are kept, passwords never.
- **Per user** (`/users/<id>`):
  - **Change role** – applies immediately, also in open sessions. The **last administrator** cannot become a viewer (checked in the same SQL statement,
    so that two simultaneous changes cannot both remove administrators). If you make yourself a viewer, you land on the overview.
  - **Language** – see below.
  - **Automatic sign-out** – tick “Exception: stay signed in” for an account that only shows pages and is never touched, e.g. a wall display
    (see [Sign-out without activity](#sign-out-without-activity)). Applies immediately, also in open sessions.
  - **Set password** – ends all sessions of the user; for your own account the current session stays signed in.
  - **Delete** – with a confirmation tick, signs out all sessions; never your own account and never the last administrator.
- Actions are logged (who, what, which user), passwords never.

## My account (`/account`, all users)

Reached through the name and role at the bottom of the sidebar. Shows user name and role, chooses **your own language** and changes **your own password**:

- You need the current password and the new one twice (10 to 128 characters, different from the current one).
- Wrong current passwords count like failed logins for the user name (`auth::throttle`); from 5 failed attempts on, requests are slowed down.
- Afterwards all other sessions of the user are signed out; the current one stays signed in.

## Language

The user interface is available in German and English. Every user has a language (column `users.language`):

- **“Same as the browser”** (default, `NULL`): the preferred language from the browser's `Accept-Language` header; if none matches, German applies.
- **Deutsch** or **English**: fixed, regardless of the browser.

It can be chosen under **My account** (your own language), when creating a user and on the user's page (administrators). The login page and error pages
before login always follow the browser. Stored names (devices, groups, sensors) are not translated.
The texts are in `locales/<language>/*.ftl` ([Fluent](https://projectfluent.org/)); notes on translating are in `docs/translations.md`.

## Creating users (command line)

The first administrator is created on the server; there are no default credentials:

```bash
opennetmon --config /etc/opennetmon/config.toml user add admin --role admin
opennetmon --config /etc/opennetmon/config.toml user add lena --role viewer --language en   # without --language: same as the browser
opennetmon --config /etc/opennetmon/config.toml user passwd lena    # ends all sessions of the user
opennetmon --config /etc/opennetmon/config.toml user list
```

The password is prompted for (twice, without echo). With input from a pipe, one line is read from stdin.
Rules: user name 3 to 32 characters (`a-z A-Z 0-9 . _ -`, case-insensitive), password 10 to 128 characters.

## Security features

- Passwords: Argon2id with a random salt. A hash is verified even for an unknown user, so that response time and message stay the same.
- Session: a random 256-bit token in the cookie `opennetmon_session` (`HttpOnly`, `SameSite=Strict`, `Secure` according to the config). The database only
  holds the SHA-256 value of the token.
- Lifetime: absolute `session_ttl_hours` (default 168 hours), and a sign-out after `session_idle_minutes` without activity (default 30, see below).
  Expired sessions are deleted when they are used and at every login.
- CSRF: the login form uses a double-submit cookie (`opennetmon_login`), all other POST forms a token per session.
- Response headers: `Content-Security-Policy` (own resources only, no `<base>` from elsewhere, no plugins), `X-Frame-Options: DENY`,
  `X-Content-Type-Options: nosniff`, `Referrer-Policy: same-origin`, and `Cache-Control: no-store` on everything but the static files (which use
  `no-cache` with an ETag), so pages are not shown again from a cache after signing out.
- Passwords never appear in logs. Failed logins are logged without the user name in the journal; the audit log (below) names the account only if it
  exists, because what was typed into the name field might be a password.

## Audit log (`/audit`, administrators only)

“Changes” in the sidebar lists who changed what and when, newest first (at most the latest 2,000 on the page, with a filter field; a click on a
user name shows only that user's entries):

- created, changed and deleted groups, devices, sensors, SNMP profiles, users, device templates, MIB files and schedules, the mail settings,
  thresholds, ratings of status values, intervals, the order of groups, devices and sensors, dependencies, maintenance assignments and applied
  templates;
- for a change the fields that differ, as “old → new” (“Name: Switch → Switch Office, Group: – → Network”); a form saved without a difference writes
  no entry;
- sign-ins, failed sign-ins (a wrong password for an existing account, or an unknown user name – the typed name is not stored) and sign-outs,
  also those after a time without activity, each with the client address (behind a proxy see `client_ip_header`).

**Secrets never go in:** for passwords, community strings and the SMTP password the entry only says “Password changed”. Pausing and resuming are
not repeated here; they are in the log (`/log`) with user and reason.

Entries are written right after the change succeeded (not in the same transaction): should writing the entry fail, the change stays and the error
is logged. They are kept for `retention_audit_days` (default 365) and stored in the table `audit_log`; texts are stored as message ids, so each
reader sees them in their own language.

## Configuration

| Key | Default | Meaning |
|---|---|---|
| `cookie_secure` | `true` | Send cookies over HTTPS only. Set it to `false` only for development over plain HTTP |
| `session_ttl_hours` | `168` | Session lifetime in hours (1 to 2160), for every user |
| `session_idle_minutes` | `30` | Sign-out after this many minutes without activity (1 to 1440; `0` switches it off) |
| `retention_audit_days` | `365` | How long entries of the audit log are kept, in days (1 to 36500) |
| `client_ip_header` | not set | Behind a reverse proxy: `"x-forwarded-for"` (last entry) or `"x-real-ip"`, so that the rate limiting sees the real client address (see below) |

## Sign-out without activity

A session ends after `session_idle_minutes` (default 30) without activity. The next request then lands on the login page with the note
“You were signed out after 30 minutes without activity”, and the session is deleted on the server.

What counts as activity:

- opening a page (a click on a link, the browser's reload, a new tab),
- sending a form,
- a click, touch, scroll wheel or key press on a page: `static/session.js` reports it to `POST /session` (with the CSRF token), at most once a minute,
  so that someone working in a long form or switching chart ranges stays signed in.

What does not count: the **automatic refresh** every 30 seconds (`static/refresh.js` marks its reloads with the short-lived cookie
`opennetmon_auto`, which `static/session.js` removes on the next page) and the requests of the scripts (chart data, table of values,
`GET /session`). An open tab that nobody looks at is therefore signed out, even with “Automatic” on.

When the time is up, `static/session.js` asks `GET /session` how much time is left (other tabs of the same session may have been active) and opens the
login page once it is over, so that no data stays on the screen. The server records activity at most every 15 seconds per session.

**Exception per user:** administrators can tick “Exception: stay signed in” on the user's page. Such accounts are never signed out for inactivity,
only after `session_ttl_hours`; use it only for displays nobody works at, and give that account the viewer role.

## Limiting login attempts (`src/auth/throttle.rs`)

Failed logins are counted in a sliding window of 15 minutes, **per user name** and **per client address**. Above a threshold, further attempts are
**rejected** (HTTP 429 with `Retry-After`, without checking the password – not even the right one), and the waiting time doubles with every further failure:

| | Threshold | Waiting time | Maximum |
|---|---|---|---|
| per user name (case-insensitive) | 5 failures | 5 s, 10 s, 20 s … | 60 s |
| per address (IPv6: per /64) | 10 failures | 30 s, 60 s, 120 s … | 15 min |

The user name is deliberately only **slowed down, never locked**: an attacker can delay the real administrator by at most a minute this way, even when
changing addresses, and gets about one password per minute. The stricter address limit prevents a single machine from trying many accounts.
A successful login resets the counters of user name and address; so does a restart of the server (the state is kept only in memory and is bounded).
The rejecting response is the same for correct and wrong user names, so accounts cannot be guessed through it.
Every attempt counts as a failure from the moment it passes the check until its password is verified, so attempts sent at the same time
are refused exactly like attempts one after the other.

At most **four password checks** run at the same time (Argon2 takes about 19 MiB each, and sign-in attempts need no account). Further attempts wait
for a free slot; after 10 seconds without one they are turned away with HTTP 503 and “try again in a few seconds”. A burst of sign-ins therefore
costs time, not the memory of the host.

**Behind a reverse proxy** the server only sees the proxy's address. With `client_ip_header = "x-forwarded-for"` (Caddy and nginx append the client address;
the *last* entry is used, i.e. the one from your own proxy) or `"x-real-ip"`, the real address is taken instead. Only set this if the service is reachable
exclusively through the proxy (default: `127.0.0.1`); otherwise a client could forge the address and bypass the address limit (the limit per user name stays
effective). Without this setting, all requests through the proxy count as one address – the strict address limit then applies to all of them together.
