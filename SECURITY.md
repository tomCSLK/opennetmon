# Security policy

## Supported versions

opennetmon is a pre-release (0.x). Security fixes go into the next release; only the latest release is supported.

## Reporting a vulnerability

Please report vulnerabilities **privately** and not as a public issue:

1. Open the [Security tab](https://github.com/tomCSLK/opennetmon/security) of the repository.
2. Choose **Report a vulnerability** and describe the problem.

Helpful details: the affected version, how to reproduce it, what an attacker can achieve and under which conditions (for example: logged in
as a reader, network access to the web interface only). Do not include real passwords or keys from your installation.

opennetmon is maintained in spare time. You will get an answer as soon as possible; once a fix is released, the advisory is published and
you are credited if you want.

## Scope and known limitations

In scope: the web interface and its authentication, the handling of stored secrets (SNMP credentials, SMTP password), the poller, the
Debian package, `deploy/install.sh` and the systemd units.

Known and documented, not vulnerabilities in themselves:

- SNMPv3 with authNoPriv or MD5 and SNMPv2c offer weak or no protection; that is a property of the protocol and the devices. Use them
  only in trusted networks ([`docs/snmp-profile.md`](docs/snmp-profile.md)).
- The built-in HTTPS uses a self-signed certificate until you replace it ([`docs/betrieb.md`](docs/betrieb.md)).
- No HSTS header (it would apply to all ports of the host).
