#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/blackpepper-dev-coexistence.XXXXXX")"
PROD_SOCKET="$TEST_ROOT/prod-tmux.sock"
DEV_SOCKET="$TEST_ROOT/dev-tmux.sock"

cleanup() {
  local status=$?
  set +e
  tmux -S "$PROD_SOCKET" kill-server 2>/dev/null || true
  tmux -S "$DEV_SOCKET" kill-server 2>/dev/null || true
  case "$TEST_ROOT" in
    "${TMPDIR:-/tmp}"/blackpepper-dev-coexistence.*) rm -rf -- "$TEST_ROOT" ;;
    *) printf 'Refusing unexpected cleanup root: %s\n' "$TEST_ROOT" >&2 ;;
  esac
  exit "$status"
}
trap cleanup EXIT HUP INT TERM

for command in git python3 tmux timeout; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'FAIL: required command is unavailable: %s\n' "$command" >&2
    exit 1
  }
done
CARGO_BIN="${CARGO:-$(command -v cargo 2>/dev/null || true)}"
[ -n "$CARGO_BIN" ] || CARGO_BIN="${HOME}/.cargo/bin/cargo"
[ -x "$CARGO_BIN" ] || {
  printf 'FAIL: cargo is not executable: %s\n' "$CARGO_BIN" >&2
  exit 1
}

WORKSPACE="$TEST_ROOT/workspace"
mkdir -p \
  "$WORKSPACE" "$TEST_ROOT/state" "$TEST_ROOT/prod-run" \
  "$TEST_ROOT/dev-run" "$TEST_ROOT/config"
chmod 0700 \
  "$WORKSPACE" "$TEST_ROOT/state" "$TEST_ROOT/prod-run" \
  "$TEST_ROOT/dev-run" "$TEST_ROOT/config"
git -C "$WORKSPACE" init --quiet --initial-branch=main
printf '%s\n' coexistence > "$WORKSPACE/README.md"
git -C "$WORKSPACE" -c user.name=e2e -c user.email=e2e@example.invalid \
  add README.md
git -C "$WORKSPACE" -c user.name=e2e -c user.email=e2e@example.invalid \
  commit --quiet -m 'test: initialize coexistence fixture'

PACKAGE_VERSION="$(sed -nE 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' \
  "$ROOT/crates/blackpepper/Cargo.toml" | head -n1)"
BUILD_ID="${PACKAGE_VERSION}-dev.coexistence-e2e"
BUILD_ROOT="$TEST_ROOT/build"
"$CARGO_BIN" build --quiet --locked --target-dir "$BUILD_ROOT" -p blackpepper --bins
PROD="$TEST_ROOT/bp"
DEV="$TEST_ROOT/bp-dev"
cp "$BUILD_ROOT/debug/bp" "$PROD"
BLACKPEPPER_BUILD_ID="$BUILD_ID" "$CARGO_BIN" build --quiet --locked \
  --target-dir "$BUILD_ROOT" -p blackpepper --bins
cp "$BUILD_ROOT/debug/bp" "$DEV"
chmod 0700 "$PROD" "$DEV"
test "$($PROD --version)" = "blackpepper $PACKAGE_VERSION"
test "$($DEV --version)" = "blackpepper $BUILD_ID"

export XDG_STATE_HOME="$TEST_ROOT/state"
export XDG_CONFIG_HOME="$TEST_ROOT/config"
export HOME="$TEST_ROOT"
export TERM=xterm-256color

tmux -S "$PROD_SOCKET" new-session -d -s production -x 80 -y 24 -c "$WORKSPACE" \
  env XDG_RUNTIME_DIR="$TEST_ROOT/prod-run" "$PROD"
tmux -S "$DEV_SOCKET" new-session -d -s development -x 80 -y 24 -c "$WORKSPACE" \
  env XDG_RUNTIME_DIR="$TEST_ROOT/dev-run" "$DEV"

for _attempt in $(seq 1 100); do
  [ -s "$TEST_ROOT/state/blackpepper/run/bp.lock" ] && \
    [ -s "$TEST_ROOT/state/blackpepper/run/bp-dev.lock" ] && break
  tmux -S "$PROD_SOCKET" has-session -t production 2>/dev/null || true
  tmux -S "$DEV_SOCKET" has-session -t development 2>/dev/null || true
  sleep 0.05
done
test -s "$TEST_ROOT/state/blackpepper/run/bp.lock"
test -s "$TEST_ROOT/state/blackpepper/run/bp-dev.lock"
test "$(cat "$TEST_ROOT/state/blackpepper/run/bp.lock")" != \
  "$(cat "$TEST_ROOT/state/blackpepper/run/bp-dev.lock")"
test -d "$TEST_ROOT/state/blackpepper/run/repository-locks"
test -d "$TEST_ROOT/state/blackpepper/run/session-locks"
python3 - "$TEST_ROOT/state/blackpepper/host-registry.sqlite3" "$WORKSPACE" <<'PY'
import sqlite3
import sys

database, workspace = sys.argv[1:]
with sqlite3.connect(database) as connection:
    local_hosts = connection.execute(
        "SELECT count(*) FROM hosts WHERE transport_json = ?", ('{"kind":"local"}',)
    ).fetchone()[0]
    workspaces = connection.execute(
        "SELECT count(*), min(root_path), max(root_path) FROM workspaces"
    ).fetchone()
if local_hosts != 1 or workspaces != (1, workspace, workspace):
    raise SystemExit(
        f"channels did not converge on one shared host/workspace: "
        f"hosts={local_hosts}, workspaces={workspaces!r}"
    )
PY

set +e
(cd "$WORKSPACE" && XDG_RUNTIME_DIR="$TEST_ROOT/prod-run" timeout 5 "$PROD") \
  >"$TEST_ROOT/prod-second.log" 2>&1
PROD_SECOND=$?
(cd "$WORKSPACE" && XDG_RUNTIME_DIR="$TEST_ROOT/dev-run" timeout 5 "$DEV") \
  >"$TEST_ROOT/dev-second.log" 2>&1
DEV_SECOND=$?
set -e
for status in "$PROD_SECOND" "$DEV_SECOND"; do
  if [ "$status" -eq 0 ] || [ "$status" -eq 124 ]; then
    printf 'FAIL: same-channel second client did not fail promptly (status %s)\n' "$status" >&2
    exit 1
  fi
done
grep -F 'Blackpepper is already running as PID' "$TEST_ROOT/prod-second.log" >/dev/null
grep -F 'Blackpepper is already running as PID' "$TEST_ROOT/dev-second.log" >/dev/null

test -f "$TEST_ROOT/state/blackpepper/host-registry.sqlite3"
test ! -e "$TEST_ROOT/state/blackpepper-dev"
printf '%s\n' 'production bp and development bp-dev coexist with separate client locks and shared registry state'
