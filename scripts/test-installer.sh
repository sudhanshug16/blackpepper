#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)"
TEST_ROOT="$(mktemp -d)"
trap 'rm -rf "$TEST_ROOT"' EXIT HUP INT TERM

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$@"
  else
    shasum -a 256 "$@"
  fi
}

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin) os="darwin" ;;
  Linux) os="linux" ;;
  *) exit 77 ;;
esac
case "$arch" in
  x86_64|amd64) arch="x86_64" ;;
  arm64|aarch64) arch="arm64" ;;
  *) exit 77 ;;
esac

asset="bp-${os}-${arch}.tar.gz"
release="$TEST_ROOT/release"
payload="$TEST_ROOT/payload"
install_dir="$TEST_ROOT/install"
mkdir -p \
  "$release" \
  "$payload/sidecars/aarch64-unknown-linux-musl" \
  "$payload/sidecars/x86_64-unknown-linux-musl"

# These positional parameters intentionally expand in the generated fixtures.
# shellcheck disable=SC2016
printf '%s\n' \
  '#!/bin/sh' \
  'if [ "${1:-}" = "--version" ]; then printf '\''blackpepper 1.2.3\n'\''; exit 0; fi' \
  'if [ "${1:-}" = "--install-dir" ]; then printf '\''%s\n'\'' "${INSTALL_DIR:-}"; exit 0; fi' \
  'exit 64' \
  > "$payload/bp"
# shellcheck disable=SC2016
printf '%s\n' \
  '#!/bin/sh' \
  'if [ "${1:-}" = "--version" ]; then printf '\''bp-host 1.2.3\n'\''; exit 0; fi' \
  'exit 64' \
  > "$payload/bp-host"
chmod 755 "$payload/bp" "$payload/bp-host"
install -m 755 "$payload/bp-host" \
  "$payload/sidecars/aarch64-unknown-linux-musl/bp-host"
install -m 755 "$payload/bp-host" \
  "$payload/sidecars/x86_64-unknown-linux-musl/bp-host"
install -m 644 "$ROOT/LICENSE" "$payload/LICENSE"
printf '%s\n' 'Apache License 2.0 fixture' > "$payload/LICENSE-HERDR-APACHE-2.0"
printf '%s\n' 'Herdr attribution fixture' > "$payload/THIRD_PARTY_NOTICES.md"
printf '%s\n' '1.2.3' > "$payload/VERSION"
(
  cd "$payload"
  sha256 \
    LICENSE \
    LICENSE-HERDR-APACHE-2.0 \
    THIRD_PARTY_NOTICES.md \
    VERSION \
    bp \
    bp-host \
    sidecars/aarch64-unknown-linux-musl/bp-host \
    sidecars/x86_64-unknown-linux-musl/bp-host \
    > SHA256SUMS
)
tar -czf "$release/$asset" -C "$payload" \
  LICENSE \
  LICENSE-HERDR-APACHE-2.0 \
  THIRD_PARTY_NOTICES.md \
  VERSION \
  SHA256SUMS \
  bp \
  bp-host \
  sidecars/aarch64-unknown-linux-musl/bp-host \
  sidecars/x86_64-unknown-linux-musl/bp-host
(
  cd "$release"
  sha256 "$asset" > SHA256SUMS
)

BLACKPEPPER_RELEASE_BASE_URL="file://$release" \
  INSTALL_DIR="$install_dir" \
  bash "$ROOT/docs/install.sh" >/dev/null
test "$("$install_dir/bp" --version)" = 'blackpepper 1.2.3'
test "$("$install_dir/bp" --install-dir)" = "$install_dir"
test "$("$install_dir/bp-host" --version)" = 'bp-host 1.2.3'
test -x "$install_dir/.blackpepper/current/sidecars/aarch64-unknown-linux-musl/bp-host"
test -x "$install_dir/.blackpepper/current/sidecars/x86_64-unknown-linux-musl/bp-host"
cmp -s "$ROOT/LICENSE" "$install_dir/.blackpepper/current/LICENSE"
test -f "$install_dir/.blackpepper/current/LICENSE-HERDR-APACHE-2.0"
test -f "$install_dir/.blackpepper/current/THIRD_PARTY_NOTICES.md"

# Reinstalling the same immutable release reuses its verified payload.
BLACKPEPPER_RELEASE_BASE_URL="file://$release" \
  INSTALL_DIR="$install_dir" \
  bash "$ROOT/docs/install.sh" >/dev/null
test "$(find "$install_dir/.blackpepper/releases" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')" = 1
test -z "$(find "$install_dir/.blackpepper/releases" -name '.current.*' -print -quit)"

# A failure after publishing staged entrypoints but before activation restores
# the exact old launch paths and leaves the old current pointer usable.
old_current="$(readlink "$install_dir/.blackpepper/current")"
# These positional parameters intentionally expand in the generated fixture.
# shellcheck disable=SC2016
printf '%s\n' \
  '#!/bin/sh' \
  'if [ "${1:-}" = "--rollback-sentinel" ]; then printf '\''old-entrypoint\n'\''; exit 0; fi' \
  'SELF_DIR=$(CDPATH= cd -P "$(dirname "$0")" && pwd)' \
  'exec "$SELF_DIR/.blackpepper/current/bp" "$@"' \
  > "$install_dir/bp"
chmod 755 "$install_dir/bp"
old_bp_digest="$(sha256 "$install_dir/bp" | awk '{print $1}')"
old_host_target=".blackpepper/current/sidecars/x86_64-unknown-linux-musl/bp-host"
rm "$install_dir/bp-host"
ln -s "$old_host_target" "$install_dir/bp-host"

if BLACKPEPPER_TEST_FAIL_BEFORE_ACTIVATION=1 \
  BLACKPEPPER_RELEASE_BASE_URL="file://$release" \
  INSTALL_DIR="$install_dir" \
  bash "$ROOT/docs/install.sh" >"$TEST_ROOT/activation-failure.log" 2>&1; then
  printf '%s\n' 'injected pre-activation failure unexpectedly succeeded' >&2
  exit 1
fi
grep -F 'injected failure before activation' "$TEST_ROOT/activation-failure.log" >/dev/null
test "$(readlink "$install_dir/.blackpepper/current")" = "$old_current"
test "$(sha256 "$install_dir/bp" | awk '{print $1}')" = "$old_bp_digest"
test "$("$install_dir/bp" --rollback-sentinel)" = 'old-entrypoint'
test "$("$install_dir/bp" --version)" = 'blackpepper 1.2.3'
test "$(readlink "$install_dir/bp-host")" = "$old_host_target"
test "$("$install_dir/bp-host" --version)" = 'bp-host 1.2.3'
test -z "$(find "$install_dir" -maxdepth 1 \
  \( -name '.bp-install.*' -o -name '.bp-host-install.*' \
  -o -name '.bp-rollback.*' -o -name '.bp-host-rollback.*' \) \
  -print -quit)"
test -z "$(find "$install_dir/.blackpepper" -maxdepth 1 \
  \( -name '.current.*' -o -name '.current-rollback.*' \) -print -quit)"

# A normal retry repairs the managed entrypoints without creating a duplicate
# immutable release.
BLACKPEPPER_RELEASE_BASE_URL="file://$release" \
  INSTALL_DIR="$install_dir" \
  bash "$ROOT/docs/install.sh" >/dev/null
test "$(find "$install_dir/.blackpepper/releases" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')" = 1
test "$("$install_dir/bp" --version)" = 'blackpepper 1.2.3'
test "$("$install_dir/bp-host" --version)" = 'bp-host 1.2.3'

# A locally damaged immutable payload is never reused; reinstall selects a
# separately staged repair bundle and leaves the damaged copy untouched.
printf 'damaged' >> "$install_dir/.blackpepper/current/LICENSE"
BLACKPEPPER_RELEASE_BASE_URL="file://$release" \
  INSTALL_DIR="$install_dir" \
  bash "$ROOT/docs/install.sh" >/dev/null
test "$(find "$install_dir/.blackpepper/releases" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')" = 2
test "$("$install_dir/bp" --version)" = 'blackpepper 1.2.3'
case "$(readlink "$install_dir/.blackpepper/current")" in
  *-repair-*) ;;
  *) printf '%s\n' 'repair payload was staged but not selected' >&2; exit 1 ;;
esac
test -z "$(find "$install_dir/.blackpepper/releases" -name '.current.*' -print -quit)"

# A changed archive with the old release checksum must fail before installation.
printf 'tampered' >> "$release/$asset"
if BLACKPEPPER_RELEASE_BASE_URL="file://$release" \
  INSTALL_DIR="$TEST_ROOT/tampered-install" \
  bash "$ROOT/docs/install.sh" >"$TEST_ROOT/tamper.log" 2>&1; then
  printf '%s\n' 'tampered release unexpectedly installed' >&2
  exit 1
fi
grep -F 'checksum mismatch' "$TEST_ROOT/tamper.log" >/dev/null
test ! -e "$TEST_ROOT/tampered-install"

printf '%s\n' 'installer atomic activation, idempotence, repair, and tamper checks passed'
