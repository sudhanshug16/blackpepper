#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/blackpepper-dev-coexistence.XXXXXX")"
PROD_SOCKET="$TEST_ROOT/prod-tmux.sock"
DEV_SOCKET="$TEST_ROOT/dev-tmux.sock"
PROD_FIRST_LOG="$TEST_ROOT/prod-first.log"
DEV_FIRST_LOG="$TEST_ROOT/dev-first.log"
PROD_SECOND_LOG="$TEST_ROOT/prod-second.log"
DEV_SECOND_LOG="$TEST_ROOT/dev-second.log"

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

dump_log() {
  local label=$1
  local path=$2
  [ -e "$path" ] || return 0
  printf '%s\n' "--- $label (last 16 KiB) ---" >&2
  if [ -s "$path" ]; then
    tail -c 16384 "$path" >&2
    printf '\n' >&2
  else
    printf '%s\n' '(empty)' >&2
  fi
}

dump_pane() {
  local label=$1
  local socket=$2
  local session=$3
  timeout 2 tmux -S "$socket" has-session -t "$session" 2>/dev/null || return 0
  printf '%s\n' "--- $label tmux pane (last 80 lines) ---" >&2
  timeout 2 tmux -S "$socket" list-panes -t "$session" \
    -F 'dead=#{pane_dead} pid=#{pane_pid} command=#{pane_current_command} status=#{pane_dead_status}' \
    >&2 || true
  timeout 2 tmux -S "$socket" capture-pane -p -S -80 -t "${session}:0.0" \
    2>/dev/null | tail -n 80 >&2 || true
}

dump_failure_context() {
  if [ -e "$PROD_SOCKET" ] || [ -e "$DEV_SOCKET" ] || \
    [ -e "$PROD_FIRST_LOG" ] || [ -e "$DEV_FIRST_LOG" ] || \
    [ -e "$PROD_SECOND_LOG" ] || [ -e "$DEV_SECOND_LOG" ]; then
    printf 'Failure diagnostics from %s (removed after this dump):\n' "$TEST_ROOT" >&2
    dump_pane 'production' "$PROD_SOCKET" production
    dump_pane 'development' "$DEV_SOCKET" development
    dump_log 'production first client stderr' "$PROD_FIRST_LOG"
    dump_log 'development first client stderr' "$DEV_FIRST_LOG"
    dump_log 'production second client' "$PROD_SECOND_LOG"
    dump_log 'development second client' "$DEV_SECOND_LOG"
  fi
}

cleanup() {
  local status=$?
  trap - EXIT HUP INT TERM
  set +e
  if [ "$status" -ne 0 ]; then
    dump_failure_context
  fi
  tmux -S "$PROD_SOCKET" kill-server 2>/dev/null || true
  tmux -S "$DEV_SOCKET" kill-server 2>/dev/null || true
  case "$TEST_ROOT" in
    "${TMPDIR:-/tmp}"/blackpepper-dev-coexistence.*) rm -rf -- "$TEST_ROOT" ;;
    *) printf 'Refusing unexpected cleanup root: %s\n' "$TEST_ROOT" >&2 ;;
  esac
  exit "$status"
}
trap cleanup EXIT
# Preserve signal failures instead of letting the shared cleanup trap inherit
# the status of whichever command happened to be interrupted.
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

start_client() {
  local socket=$1
  local session=$2
  local runtime_root=$3
  local binary=$4
  local stderr_log=$5
  # Keep a failed pane long enough for cleanup to capture it. Start an idle
  # shell first so remain-on-exit is armed before Blackpepper can fail.
  tmux -S "$socket" new-session -d -s "$session" -x 80 -y 24 -c "$WORKSPACE"
  tmux -S "$socket" set-option -w -t "${session}:0" remain-on-exit on
  # shellcheck disable=SC2016 # The child shell expands its positional arguments.
  tmux -S "$socket" respawn-pane -k -t "${session}:0.0" -c "$WORKSPACE" \
    env XDG_RUNTIME_DIR="$runtime_root" sh -c 'exec "$1" 2>"$2"' \
    blackpepper-coexistence "$binary" "$stderr_log"
}

live_lock_pid() {
  local lock_path=$1
  local pid
  [ -s "$lock_path" ] || return 1
  pid="$(<"$lock_path")"
  case "$pid" in
    '' | *[!0-9]*) return 1 ;;
  esac
  kill -0 "$pid" 2>/dev/null || return 1
  printf '%s\n' "$pid"
}

pane_is_live() {
  local socket=$1
  local session=$2
  [ "$(tmux -S "$socket" display-message -p -t "${session}:0.0" \
    '#{pane_dead}' 2>/dev/null)" = 0 ]
}

client_is_live() {
  local socket=$1
  local session=$2
  local lock_path=$3
  live_lock_pid "$lock_path" >/dev/null && pane_is_live "$socket" "$session"
}

registry_is_converged() {
  local database=$1
  local workspace=$2
  [ -s "$database" ] || return 1
  python3 - "$database" "$workspace" 2>/dev/null <<'PY'
import sqlite3
import sys

database, workspace = sys.argv[1:]
try:
    with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
        local_hosts = connection.execute(
            "SELECT count(*) FROM hosts WHERE transport_json = ?", ('{"kind":"local"}',)
        ).fetchone()[0]
        workspaces = connection.execute(
            "SELECT count(*), min(root_path), max(root_path) FROM workspaces"
        ).fetchone()
except sqlite3.Error:
    raise SystemExit(1)
raise SystemExit(0 if local_hosts == 1 and workspaces == (1, workspace, workspace) else 1)
PY
}

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
[ -n "$PACKAGE_VERSION" ] || fail 'could not read the Blackpepper package version'
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
PROD_VERSION="$("$PROD" --version)" || fail 'production test binary did not report its version'
DEV_VERSION="$("$DEV" --version)" || fail 'development test binary did not report its version'
[ "$PROD_VERSION" = "blackpepper $PACKAGE_VERSION" ] || \
  fail "production version mismatch: expected 'blackpepper $PACKAGE_VERSION', got '$PROD_VERSION'"
[ "$DEV_VERSION" = "blackpepper $BUILD_ID" ] || \
  fail "development version mismatch: expected 'blackpepper $BUILD_ID', got '$DEV_VERSION'"

export XDG_STATE_HOME="$TEST_ROOT/state"
export XDG_CONFIG_HOME="$TEST_ROOT/config"
export HOME="$TEST_ROOT"
export TERM=xterm-256color

start_client "$PROD_SOCKET" production "$TEST_ROOT/prod-run" "$PROD" "$PROD_FIRST_LOG"
start_client "$DEV_SOCKET" development "$TEST_ROOT/dev-run" "$DEV" "$DEV_FIRST_LOG"

PROD_LOCK="$TEST_ROOT/state/blackpepper/run/bp.lock"
DEV_LOCK="$TEST_ROOT/state/blackpepper/run/bp-dev.lock"
REGISTRY="$TEST_ROOT/state/blackpepper/host-registry.sqlite3"
for _attempt in $(seq 1 100); do
  client_is_live "$PROD_SOCKET" production "$PROD_LOCK" && \
    client_is_live "$DEV_SOCKET" development "$DEV_LOCK" && \
    registry_is_converged "$REGISTRY" "$WORKSPACE" && break
  sleep 0.05
done
PROD_PID="$(live_lock_pid "$PROD_LOCK")" || \
  fail 'production client did not retain a live singleton lock owner'
DEV_PID="$(live_lock_pid "$DEV_LOCK")" || \
  fail 'development client did not retain a live singleton lock owner'
pane_is_live "$PROD_SOCKET" production || fail 'production tmux client exited during startup'
pane_is_live "$DEV_SOCKET" development || fail 'development tmux client exited during startup'
[ "$PROD_PID" != "$DEV_PID" ] || \
  fail "production and development lock files reported the same PID $PROD_PID"
[ -d "$TEST_ROOT/state/blackpepper/run/repository-locks" ] || \
  fail 'shared repository-lock directory was not created'
[ -d "$TEST_ROOT/state/blackpepper/run/session-locks" ] || \
  fail 'shared session-lock directory was not created'
python3 - "$REGISTRY" "$WORKSPACE" <<'PY'
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
  >"$PROD_SECOND_LOG" 2>&1
PROD_SECOND=$?
(cd "$WORKSPACE" && XDG_RUNTIME_DIR="$TEST_ROOT/dev-run" timeout 5 "$DEV") \
  >"$DEV_SECOND_LOG" 2>&1
DEV_SECOND=$?
set -e
for channel_status in "production:$PROD_SECOND" "development:$DEV_SECOND"; do
  channel=${channel_status%%:*}
  status=${channel_status#*:}
  if [ "$status" -eq 0 ] || [ "$status" -eq 124 ]; then
    fail "$channel same-channel second client did not fail promptly (status $status)"
  fi
done
grep -Fq 'Blackpepper is already running as PID' "$PROD_SECOND_LOG" || \
  fail 'production second client did not report the live singleton owner'
grep -Fq 'Blackpepper is already running as PID' "$DEV_SECOND_LOG" || \
  fail 'development second client did not report the live singleton owner'
client_is_live "$PROD_SOCKET" production "$PROD_LOCK" || \
  fail 'production first client exited during the same-channel refusal check'
client_is_live "$DEV_SOCKET" development "$DEV_LOCK" || \
  fail 'development first client exited during the same-channel refusal check'

[ -f "$REGISTRY" ] || \
  fail 'shared host registry was not created'
[ ! -e "$TEST_ROOT/state/blackpepper-dev" ] || \
  fail 'development client created a forbidden separate state tree'
printf '%s\n' 'production bp and development bp-dev coexist with separate client locks and shared registry state'
