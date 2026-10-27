#!/usr/bin/env bash
# Builds, checks and (optionally) publishes a GitHub release of opennetmon: a tar.gz archive and a .deb package.
#
#   dev/release.sh                                  # build and check only; result in target/dist/
#   dev/release.sh --publish --notes notes.md       # additionally tag vX.Y.Z, push the tag, create the release
#   dev/release.sh --publish --notes notes.md --prerelease   # the same, marked as pre-release (e.g. for a test version)
#
# The version comes from Cargo.toml (bump it there and commit first). Before anything is published the script
# requires a clean work tree on main that matches origin/main, a version without an existing tag, passing
# fmt/clippy/tests, no RustSec advisories (cargo audit), an up-to-date THIRD-PARTY-LICENSES.txt, a successful
# installer dry run from the archive and the expected files in the .deb (built with dev/deb.sh, needs cargo-deb).
# Releases are regular releases since 0.4.4 (0.1.0 to 0.4.3 were pre-releases); --prerelease marks one as pre-release.
set -euo pipefail

cd "$(dirname "$0")/.."
TARGET=x86_64-unknown-linux-musl
PUBLISH=no
NOTES=""
PRERELEASE=()
while [ $# -gt 0 ]; do
  case "$1" in
    --publish) PUBLISH=yes; shift ;;
    --notes) NOTES="${2:?--notes needs a file}"; shift 2 ;;
    --prerelease) PRERELEASE=(--prerelease); shift ;;
    -h|--help) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

step() { printf '==> %s\n' "$*"; }
fail() { printf 'error: %s\n' "$*" >&2; exit 1; }

VERSION=$(cargo metadata --format-version 1 --no-deps | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')
TAG="v$VERSION"
NAME="opennetmon-$VERSION-x86_64-linux"
DEB="opennetmon_$VERSION-1_amd64.deb"
DIST="target/dist"

step "checking the repository for $TAG"
if [ "$PUBLISH" = yes ]; then
  [ -n "$NOTES" ] && [ -f "$NOTES" ] || fail "--publish needs --notes FILE with the release notes"
  [ -z "$(git status --porcelain)" ] || fail "work tree is not clean"
  [ "$(git rev-parse --abbrev-ref HEAD)" = main ] || fail "not on main"
  git fetch -q origin main
  [ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || fail "main differs from origin/main (push or pull first)"
  git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && fail "tag $TAG already exists locally (bump the version in Cargo.toml)"
  if git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null; then fail "tag $TAG already exists on GitHub"; fi
elif [ -n "$(git status --porcelain)" ]; then
  echo "note: uncommitted changes; the archive takes deploy/ and docs/ from HEAD, the rest from the work tree"
fi

step "checking code and licenses"
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --quiet
python3 -I dev/third-party-licenses.py --check
command -v cargo-audit >/dev/null || fail "cargo-audit is missing (cargo install cargo-audit --locked)"
cargo audit --quiet

step "building the static binary ($TARGET)"
cargo build --release --target "$TARGET"
BINARY="target/$TARGET/release/opennetmon"
[ "$("$BINARY" --version)" = "opennetmon $VERSION" ] || fail "binary reports $("$BINARY" --version), expected $VERSION"

step "packaging $DIST/$NAME.tar.gz"
rm -rf "$DIST" && mkdir -p "$DIST/$NAME"
cp "$BINARY" LICENSE THIRD-PARTY-LICENSES.txt README.md "$DIST/$NAME/"
git archive HEAD deploy docs | tar -x -C "$DIST/$NAME"
(cd "$DIST" && tar --owner=0 --group=0 -czf "$NAME.tar.gz" "$NAME")

step "packaging $DIST/$DEB"
dev/deb.sh --no-build --quiet >/dev/null
cp "target/debian/$DEB" "$DIST/"
CONTENTS=$(dpkg-deb --contents "$DIST/$DEB")
for f in usr/bin/opennetmon usr/sbin/opennetmon-backup etc/opennetmon/config.toml usr/lib/systemd/system/opennetmon.service \
         usr/lib/systemd/system/opennetmon-backup.timer usr/share/doc/opennetmon/copyright \
         usr/share/doc/opennetmon/THIRD-PARTY-LICENSES.txt; do
  grep -q " \./$f\$" <<<"$CONTENTS" || fail "$DEB does not contain /$f"
done
[ "$(dpkg-deb --field "$DIST/$DEB" Version)" = "$VERSION-1" ] || fail "$DEB has the wrong version"
(cd "$DIST" && sha256sum "$NAME.tar.gz" "$DEB" > SHA256SUMS)

step "installer dry run from the unpacked archive"
CHECK="$DIST/check"
mkdir -p "$CHECK" && tar -xzf "$DIST/$NAME.tar.gz" -C "$CHECK"
(cd "$CHECK/$NAME" && DESTDIR="$PWD/../root" ./deploy/install.sh --binary ./opennetmon >/dev/null)
for f in usr/local/bin/opennetmon etc/systemd/system/opennetmon.service usr/local/share/doc/opennetmon/LICENSE \
         usr/local/share/doc/opennetmon/THIRD-PARTY-LICENSES.txt etc/opennetmon/tls-cert.pem etc/opennetmon/tls-key.pem; do
  [ -e "$CHECK/root/$f" ] || fail "dry run did not install /$f"
done
rm -rf "$CHECK" "$DIST/$NAME"
cat "$DIST/SHA256SUMS"

if [ "$PUBLISH" = no ]; then
  step "done (not published; add --publish --notes FILE to tag and release $TAG)"
  exit 0
fi

step "tagging $TAG and creating the GitHub release"
git tag -a "$TAG" -m "opennetmon $VERSION"
git push -q origin "$TAG"
gh release create "$TAG" --verify-tag "${PRERELEASE[@]}" --title "opennetmon $VERSION" --notes-file "$NOTES" \
  "$DIST/$NAME.tar.gz" "$DIST/$DEB" "$DIST/SHA256SUMS"

step "verifying the uploaded files"
VERIFY=$(mktemp -d)
trap 'rm -rf "$VERIFY"' EXIT
gh release download "$TAG" --dir "$VERIFY"
(cd "$VERIFY" && sha256sum -c SHA256SUMS)
step "published $TAG"
