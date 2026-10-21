<!-- Thanks for contributing! Please read CONTRIBUTING.md first. -->

## What and why

## Checklist

- [ ] Every commit is signed off (`git commit -s`, see the DCO section in CONTRIBUTING.md)
- [ ] `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass
- [ ] UI texts only through `locales/de/*.ftl` and `locales/en/*.ftl` (both languages)
- [ ] `THIRD-PARTY-LICENSES.txt` regenerated if `Cargo.lock` changed (`python3 -I dev/third-party-licenses.py`)
- [ ] Docs updated where behaviour changed (`docs/` is German, `README.md` English)
