# Contributing to opennetmon

Thanks for your interest! opennetmon works and is tested on real hardware, but so far mostly with devices of one vendor (TP-Link Omada).
Right now the most valuable contribution is **testing it** with your devices and telling us what breaks.

## Reporting bugs and ideas

- **Bugs:** open an issue with the bug report form. Include the version, how you installed it, your OS and, for SNMP problems, the device
  (vendor, model, firmware, SNMP version). Never paste passwords, communities or `secret.key`; replace them with `***`.
- **Ideas and sensor requests:** use the feature request form. Mention the MIB or OIDs if you know them.
- **Security vulnerabilities:** please do **not** open a public issue, see [`SECURITY.md`](SECURITY.md).

For a larger change, open an issue first so we can agree on the approach before you invest the time.

## Development

Requirement: a current stable Rust toolchain (edition 2024). Getting started is described in the README under
[Quick start (development)](README.md#quick-start-development). Before you open a pull request, all of these must pass:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests need no network (a fake SNMP agent is built in). Optional tests against a local `snmpd` are described in
[`docs/dev-testagent.md`](docs/dev-testagent.md). CI runs the same checks on every pull request, plus `cargo audit`
(known vulnerabilities in dependencies from the RustSec database; exceptions with a reason in `.cargo/audit.toml`), which also runs weekly.
If you change `Cargo.lock`, run it locally too (`cargo install cargo-audit --locked`).

## Conventions

- **Language:** code, identifiers, code comments and commit messages are English. The documentation in `docs/` and `README.md` is English;
  only the internal planning files `PLAN.md` and `CLAUDE.md` are German.
- **UI texts** only through the Fluent catalogs in `locales/<code>/*.ftl`, never as fixed text in templates, Rust or JavaScript.
  German (`de`) is the source language; add every new message in English (`en`) too. Tests check that both catalogs match the code.
- **No inline styles or scripts:** the Content Security Policy forbids them. Styles go into `static/app.css`, scripts into `static/`.
- **No `unwrap()`/`expect()`** outside tests and startup code; errors go through the module's error type.
- **New sensor types** only through the `Sensor` trait, no special cases in the poller.
- **SNMP stays read-only.** Secrets never go into logs, HTML or the repository.
- **Dependencies:** only maintained crates with a license compatible with the AGPL (MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0 and similar).
  Give a short reason in the commit message. After any change to `Cargo.lock`, run `python3 -I dev/third-party-licenses.py` and commit
  `THIRD-PARTY-LICENSES.txt`.
- Small, self-contained commits; every commit should build and pass the tests.

## Developer Certificate of Origin (DCO)

opennetmon uses the [Developer Certificate of Origin 1.1](https://developercertificate.org/) instead of a contributor license agreement.
By signing off a commit you certify that you wrote the change, or otherwise have the right to submit it under the project's license.

Sign off every commit by adding a line with your name and the e-mail address of the commit author:

```
Signed-off-by: Jane Doe <jane@example.org>
```

`git commit -s` adds it for you. To sign off commits you already made: `git commit --amend -s` for the last one, or
`git rebase --signoff main` for all commits of your branch, then force-push the branch. CI checks every commit of a pull request.

## License

opennetmon is licensed under the [GNU Affero General Public License v3.0 or later](LICENSE). By contributing, you agree that your
contribution is licensed under the same license.
