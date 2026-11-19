#!/usr/bin/env bash
# Builds the container image of opennetmon for the architecture of this machine and checks it (docs/docker.md).
#
#   dev/container.sh                       # build the static binary, then the image opennetmon:<version>
#   dev/container.sh --binary FILE         # use this binary instead (the release workflow builds it before)
#   dev/container.sh --test                # also run the smoke test against the new image
#   dev/container.sh --test-only IMAGE     # only run the smoke test against IMAGE
#   dev/container.sh --tag NAME            # image name (default opennetmon:<version>)
#
# Uses podman if present, else docker (ENGINE=docker dev/container.sh to choose). The image is built from a prepared
# context in target/container/ (Containerfile, rootfs/, state/) without RUN steps. Nothing is pushed.
set -euo pipefail

cd "$(dirname "$0")/.."
BINARY=""
TEST=no
BUILD=yes
IMAGE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --binary) BINARY="${2:?--binary needs a file}"; shift 2 ;;
    --test) TEST=yes; shift ;;
    --test-only) TEST=yes; BUILD=no; IMAGE="${2:?--test-only needs an image}"; shift 2 ;;
    --tag) IMAGE="${2:?--tag needs a name}"; shift 2 ;;
    -h|--help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

step() { printf '==> %s\n' "$*"; }
fail() { printf 'error: %s\n' "$*" >&2; exit 1; }

ENGINE="${ENGINE:-$(command -v podman >/dev/null && echo podman || echo docker)}"
command -v "$ENGINE" >/dev/null || fail "neither podman nor docker found"
VERSION=$(cargo metadata --format-version 1 --no-deps | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')
IMAGE="${IMAGE:-opennetmon:$VERSION}"

if [ "$BUILD" = yes ]; then
  case "$(uname -m)" in
    x86_64) TARGET=x86_64-unknown-linux-musl ;;
    aarch64|arm64) TARGET=aarch64-unknown-linux-musl ;;
    *) fail "no musl target for $(uname -m)" ;;
  esac
  if [ -z "$BINARY" ]; then
    step "building the static binary ($TARGET)"
    cargo build --release --locked --target "$TARGET"
    BINARY="target/$TARGET/release/opennetmon"
  fi
  [ "$("$BINARY" --version)" = "opennetmon $VERSION" ] || fail "$BINARY reports $("$BINARY" --version), expected $VERSION"
  file -b "$BINARY" | grep -Eq "statically linked|static-pie linked" || fail "$BINARY is not statically linked"

  step "preparing the build context"
  CONTEXT=target/container
  rm -rf "$CONTEXT"
  mkdir -p "$CONTEXT/rootfs/usr/bin" "$CONTEXT/rootfs/usr/share/doc/opennetmon" "$CONTEXT/rootfs/tmp" \
           "$CONTEXT/state/etc/opennetmon" "$CONTEXT/state/var/lib/opennetmon"
  install -m 0755 "$BINARY" "$CONTEXT/rootfs/usr/bin/opennetmon"
  install -m 0644 LICENSE THIRD-PARTY-LICENSES.txt "$CONTEXT/rootfs/usr/share/doc/opennetmon/"
  chmod 1777 "$CONTEXT/rootfs/tmp"
  chmod 0700 "$CONTEXT/state/etc/opennetmon" "$CONTEXT/state/var/lib/opennetmon"
  cp Containerfile "$CONTEXT/"

  step "building $IMAGE with $ENGINE"
  FORMAT=()
  # Podman writes OCI images by default, which have no HEALTHCHECK; the Docker format keeps it.
  [ "$(basename "$ENGINE")" = podman ] && FORMAT=(--format docker)
  "$ENGINE" build "${FORMAT[@]}" --file "$CONTEXT/Containerfile" --tag "$IMAGE" \
    --build-arg "VERSION=$VERSION" \
    --build-arg "REVISION=$(git rev-parse HEAD)" \
    --build-arg "CREATED=$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    "$CONTEXT"
  "$ENGINE" image inspect --format '{{.Size}}' "$IMAGE" | awk '{ printf "    image size: %.1f MB\n", $1 / 1000000 }'
fi

[ "$TEST" = yes ] || exit 0

# Smoke test: start the image as compose.yaml does (read-only, no capabilities), wait until it is healthy, create an
# administrator, sign in over HTTPS and check the main page; then restart and check that key and data stayed.
step "smoke test of $IMAGE"
NAME="opennetmon-smoke-$$"
PORT="${SMOKE_PORT:-18443}"
WORK=$(mktemp -d)
cleanup() {
  "$ENGINE" rm -f "$NAME" >/dev/null 2>&1 || true
  "$ENGINE" volume rm -f "$NAME-config" "$NAME-data" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

start() {
  "$ENGINE" run -d --name "$NAME" -p "127.0.0.1:$PORT:8443" \
    -v "$NAME-config:/etc/opennetmon" -v "$NAME-data:/var/lib/opennetmon" \
    -e TZ=Europe/Berlin --sysctl net.ipv4.ping_group_range="0 65535" \
    --read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges:true \
    "$IMAGE" >/dev/null
}

wait_healthy() {
  for _ in $(seq 1 30); do
    if "$ENGINE" exec "$NAME" opennetmon healthcheck >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  "$ENGINE" logs "$NAME" >&2
  fail "the container did not become healthy"
}

start
wait_healthy
echo "    healthy"
# The log goes into a variable first: `grep -q` would end the pipe early and fail it under pipefail.
LOG=$("$ENGINE" logs "$NAME" 2>&1)
grep -q "created a self-signed certificate" <<<"$LOG" || fail "no certificate was created"
grep -q 'time_zone=Europe/Berlin' <<<"$LOG" || fail "the time zone from TZ is not used"

PASSWORD="smoke-$(head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n')"
printf '%s\n' "$PASSWORD" | "$ENGINE" exec -i "$NAME" opennetmon user add smoke --role admin >/dev/null
USERS=$("$ENGINE" exec "$NAME" opennetmon user list)
grep -q '^smoke ' <<<"$USERS" || fail "user add did not create the user"

login() {
  local jar="$WORK/cookies" page token
  rm -f "$jar"
  page=$(curl -sk -c "$jar" "https://127.0.0.1:$PORT/login")
  token=$(sed -n 's/.*name="csrf_token" value="\([^"]*\)".*/\1/p' <<<"$page" | head -n1)
  [ -n "$token" ] || fail "no sign-in form"
  curl -sk -b "$jar" -c "$jar" -o /dev/null --data-urlencode "username=smoke" --data-urlencode "password@-" \
    --data-urlencode "csrf_token=$token" "https://127.0.0.1:$PORT/login" < <(printf '%s' "$PASSWORD")
  page=$(curl -sk -b "$jar" "https://127.0.0.1:$PORT/")
  grep -q 'smoke' <<<"$page" || fail "sign-in failed"
}
login
echo "    signed in"
FINGERPRINT=$("$ENGINE" exec "$NAME" opennetmon tls generate 2>&1 || true)
grep -q "already exists" <<<"$FINGERPRINT" || fail "tls generate did not refuse to overwrite the certificate"

"$ENGINE" restart "$NAME" >/dev/null
wait_healthy
login
LOG=$("$ENGINE" logs "$NAME" 2>&1)
[ "$(grep -c "created a new encryption key" <<<"$LOG")" = 1 ] || fail "the key was created again after a restart"
echo "    restart kept key, certificate and data"
step "smoke test passed"
