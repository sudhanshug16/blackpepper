#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)"
FIXTURES="$ROOT/scripts/fixtures/tui-local-e2e"
# shellcheck source=scripts/fixtures/tui-local-e2e/harness-lib.sh
source "$FIXTURES/harness-lib.sh"

for requirement in git id python3 tmux; do
  command -v "$requirement" >/dev/null 2>&1 || {
    printf 'FAIL: required command is unavailable: %s\n' "$requirement" >&2
    exit 1
  }
done

if [ "$(uname -s)" != Linux ]; then
  printf '%s\n' 'FAIL: the Zellij socket namespace acceptance requires Linux.' >&2
  exit 1
fi

BP_LAUNCHER="${BLACKPEPPER_ZELLIJ_NAMESPACE_E2E_BP_DEV:-$(command -v bp-dev 2>/dev/null || true)}"
if [ -z "$BP_LAUNCHER" ] || [ ! -x "$BP_LAUNCHER" ]; then
  printf '%s\n' 'FAIL: globally installed bp-dev was not found; run scripts/setup.sh first.' >&2
  exit 1
fi
if [ -z "${BLACKPEPPER_ZELLIJ_NAMESPACE_E2E_BP_DEV:-}" ]; then
  LAUNCHER_DIR="$(CDPATH='' cd -P -- "$(dirname "$BP_LAUNCHER")" && pwd)"
  CURRENT_BUNDLE="$LAUNCHER_DIR/.blackpepper-dev/current"
  [ -d "$CURRENT_BUNDLE" ] || {
    printf 'FAIL: bp-dev launcher has no installed current bundle: %s\n' "$CURRENT_BUNDLE" >&2
    exit 1
  }
  CURRENT_BUNDLE="$(CDPATH='' cd -P -- "$CURRENT_BUNDLE" && pwd)"
  BP_DEV="$CURRENT_BUNDLE/bp-dev"
else
  BP_DEV="$BP_LAUNCHER"
  CURRENT_BUNDLE="$(CDPATH='' cd -P -- "$(dirname "$BP_DEV")" && pwd)"
fi
BP_HOST="$CURRENT_BUNDLE/bp-host"
if [ ! -x "$BP_DEV" ] || [ -L "$BP_DEV" ] || [ ! -x "$BP_HOST" ] || [ -L "$BP_HOST" ]; then
  printf 'FAIL: immutable development client/helper bundle is incomplete: %s\n' "$CURRENT_BUNDLE" >&2
  exit 1
fi
BP_VERSION="$($BP_DEV --version 2>&1)"
BUILD_ID="${BP_VERSION#blackpepper }"
case "$BP_VERSION" in
  'blackpepper '*'-dev.'*) ;;
  *) printf 'FAIL: expected a development build, got: %s\n' "$BP_VERSION" >&2; exit 1 ;;
esac
[ "$($BP_HOST --version 2>&1)" = "bp-host $BUILD_ID" ] || {
  printf '%s\n' 'FAIL: installed bp-dev and bp-host build IDs do not match.' >&2
  exit 1
}

# Keep the injected hostile roots short enough that the unfixed behavior
# reaches the namespace split instead of Zellij's 107-byte Unix-socket limit.
TEST_ROOT="$(mktemp -d /tmp/bp-zns.XXXXXX)"
ARTIFACTS="$TEST_ROOT/artifacts"
WORKSPACE="$TEST_ROOT/workspace"
TEMP_HOME="$TEST_ROOT/home"
TMUX_SOCKET="$TEST_ROOT/tmux.sock"
TMUX_SESSION='bp-zellij-namespace-e2e'
SECOND_TMUX_SOCKET=''
LISTENER_PID=''
ZELLIJ_BIN=''
ZELLIJ_SOCKET_ROOT=''
BACKEND_SESSION=''
HOSTILE_SOCKET_A="$TEST_ROOT/hostile-a"
HOSTILE_SOCKET_B="$TEST_ROOT/hostile-b"
HOSTILE_TMPDIR="$TEST_ROOT/hostile-tmp"
HOSTILE_TMP_SOCKET="$HOSTILE_TMPDIR/zellij-$(id -u)"
SECOND_RUNTIME="$TEST_ROOT/second-runtime"
CANONICAL_SOCKET="/tmp/zellij-$(id -u)"
SECOND_XDG_SOCKET="$SECOND_RUNTIME/zellij"
STANDARD_XDG_SOCKET="/run/user/$(id -u)/zellij"
E2E_DATA_HOME="${BLACKPEPPER_ZELLIJ_NAMESPACE_E2E_DATA_HOME:-$ROOT/target/tui-local-e2e-data}"
NAMESPACE_TOKEN="namespace-$BUILD_ID-$$"

case "$E2E_DATA_HOME" in
  /*) ;;
  *) printf 'FAIL: namespace E2E data home must be absolute: %s\n' "$E2E_DATA_HOME" >&2; exit 1 ;;
esac

reported_session_rows_are_valid() {
  python3 - "$1" <<'PY'
from pathlib import Path
import sys

data = Path(sys.argv[1]).read_bytes()
rows = [row for row in data.split(b"\n") if row]
valid = (
    0 < len(data) <= 65_536
    and data.endswith(b"\n")
    and b"\0" not in data
    and bool(rows)
    and all(row != b"\r" for row in rows)
)
raise SystemExit(0 if valid else 1)
PY
}

stderr_is_no_active_session() {
  python3 - "$1" <<'PY'
from pathlib import Path
import sys

raise SystemExit(
    0
    if Path(sys.argv[1]).read_bytes().rstrip(b"\r\n")
    == b"There is no active session!"
    else 1
)
PY
}

namespace_probe() {
  local socket_dir="$1" output error rows status first_line missing_header
  output="$TEST_ROOT/probe.stdout"
  error="$TEST_ROOT/probe.stderr"
  rows="$TEST_ROOT/probe.rows"
  missing_header="Session '$BACKEND_SESSION' not found. The following sessions are active:"
  set +e
  ZELLIJ_SOCKET_DIR="$socket_dir" "$ZELLIJ_BIN" \
    --session "$BACKEND_SESSION" action list-clients >"$output" 2>"$error"
  status=$?
  set -e
  first_line="$(sed -n '1p' "$error")"
  if [ "$status" -eq 0 ]; then
    if [ "$first_line" = "$missing_header" ] &&
      [ "$(wc -l < "$error")" -eq 1 ] &&
      reported_session_rows_are_valid "$output"; then
      printf '%s\n' missing
      return 0
    fi
    [ ! -s "$error" ] || {
      LAST_CAPTURE="$error"
      fail_e2e "Zellij reported unexpected stderr while probing namespace $socket_dir"
    }
    IFS= read -r first_line < "$output" || true
    [ "$first_line" = 'CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND' ] || {
      LAST_CAPTURE="$output"
      fail_e2e "Zellij returned an invalid client list in namespace $socket_dir"
    }
    printf '%s\n' active
    return 0
  fi
  if [ "$status" -eq 1 ] && [ ! -s "$output" ]; then
    if stderr_is_no_active_session "$error"; then
      printf '%s\n' missing
      return 0
    fi
    tail -n +2 "$error" > "$rows"
    if [ "$first_line" = "$missing_header" ] &&
      reported_session_rows_are_valid "$rows"; then
      printf '%s\n' missing
      return 0
    fi
  fi
  LAST_CAPTURE="$error"
  fail_e2e "Zellij namespace probe failed unexpectedly for $socket_dir (status $status)"
}

session_from_registry() {
  local require_one="${1:-yes}"
  python3 - "$TEST_ROOT/state/blackpepper/host-registry.sqlite3" \
    "$WORKSPACE" "$require_one" <<'PY'
import sqlite3
import sys

database, workspace, require_one = sys.argv[1:]
with sqlite3.connect(database) as connection:
    rows = connection.execute(
        """
        SELECT sessions.backend_session_id
        FROM sessions
        JOIN workspaces ON workspaces.id = sessions.workspace_id
        WHERE workspaces.root_path = ?
          AND sessions.backend_json = '{"kind":"zellij"}'
          AND sessions.state_json != '"exited"'
        """,
        (workspace,),
    ).fetchall()
if require_one == "yes" and len(rows) != 1:
    raise SystemExit(f"expected one registered active session, got {rows!r}")
if len(rows) == 1:
    print(rows[0][0])
PY
}

valid_backend_session() {
  printf '%s\n' "$1" | grep -Eq \
    '^bp-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
}

assert_single_canonical_namespace() {
  local label="$1" attempts="${2:-150}" canonical candidate state
  for _attempt in $(seq 1 "$attempts"); do
    canonical="$(namespace_probe "$CANONICAL_SOCKET")"
    for candidate in \
      "$HOSTILE_SOCKET_A" "$HOSTILE_SOCKET_B" "$HOSTILE_TMP_SOCKET" \
      "$SECOND_XDG_SOCKET"; do
      state="$(namespace_probe "$candidate")"
      [ "$state" = missing ] ||
        fail_e2e "$label duplicated the registered Zellij session in $candidate"
    done
    if [ -d "${STANDARD_XDG_SOCKET%/zellij}" ]; then
      state="$(namespace_probe "$STANDARD_XDG_SOCKET")"
      [ "$state" = missing ] ||
        fail_e2e "$label duplicated the registered Zellij session in $STANDARD_XDG_SOCKET"
    fi
    [ "$canonical" = active ] && return 0
    [ "$_attempt" -eq "$attempts" ] || sleep 0.1
  done
  fail_e2e "$label split the registered Zellij session across socket namespaces"
}

wait_for_canonical_client_count() {
  local expected="$1" output count
  for _attempt in $(seq 1 150); do
    output="$(ZELLIJ_SOCKET_DIR="$CANONICAL_SOCKET" "$ZELLIJ_BIN" \
      --session "$BACKEND_SESSION" action list-clients 2>/dev/null || true)"
    count="$(printf '%s\n' "$output" | awk 'NR > 1 && NF { count += 1 } END { print count + 0 }')"
    [ "$count" -eq "$expected" ] && return 0
    sleep 0.1
  done
  fail_e2e "canonical Zellij session did not reach $expected controlling client(s)"
}

cleanup_namespace_e2e() {
  local status=$? discovered=''
  set +e
  if tmux -S "$TMUX_SOCKET" has-session -t "$TMUX_SESSION" 2>/dev/null; then
    tmux -S "$TMUX_SOCKET" kill-server >/dev/null 2>&1
  fi
  if [ -z "$BACKEND_SESSION" ] && [ -f "$TEST_ROOT/state/blackpepper/host-registry.sqlite3" ]; then
    discovered="$(session_from_registry no 2>/dev/null || true)"
    valid_backend_session "$discovered" && BACKEND_SESSION="$discovered"
  fi
  if [ -z "$ZELLIJ_BIN" ]; then
    ZELLIJ_BIN="$(find "$E2E_DATA_HOME/blackpepper/sidecars/zellij/0.44.3" \
      -type f -name zellij -perm -u+x -print -quit 2>/dev/null || true)"
  fi
  if [ -n "$ZELLIJ_BIN" ] && [ -x "$ZELLIJ_BIN" ] && [ -n "$BACKEND_SESSION" ]; then
    for socket_dir in \
      "$CANONICAL_SOCKET" "$HOSTILE_SOCKET_A" "$HOSTILE_SOCKET_B" \
      "$HOSTILE_TMP_SOCKET" "$SECOND_XDG_SOCKET" "$STANDARD_XDG_SOCKET"; do
      ZELLIJ_SOCKET_DIR="$socket_dir" "$ZELLIJ_BIN" kill-session "$BACKEND_SESSION" \
        >/dev/null 2>&1 || true
    done
  fi
  if [ "$status" -ne 0 ] && [ "${BLACKPEPPER_ZELLIJ_NAMESPACE_E2E_KEEP:-0}" = 1 ]; then
    printf 'Preserved failed namespace E2E: %s\n' "$TEST_ROOT" >&2
  else
    case "$TEST_ROOT" in
      /tmp/bp-zns.*) rm -rf -- "$TEST_ROOT" ;;
      *) printf 'Refusing to clean unexpected test root: %s\n' "$TEST_ROOT" >&2 ;;
    esac
  fi
  exit "$status"
}
trap cleanup_namespace_e2e EXIT HUP INT TERM

install -d -m 0700 \
  "$ARTIFACTS" "$WORKSPACE" "$TEMP_HOME" "$TEST_ROOT/config" \
  "$TEST_ROOT/state" "$TEST_ROOT/cache" "$HOSTILE_SOCKET_A" \
  "$HOSTILE_SOCKET_B" "$HOSTILE_TMPDIR" "$SECOND_RUNTIME" "$E2E_DATA_HOME"
printf '%s\n' '# Blackpepper Zellij namespace acceptance fixture' > "$WORKSPACE/README.md"
git -C "$WORKSPACE" init --initial-branch=main --quiet
git -C "$WORKSPACE" config user.name 'Blackpepper E2E'
git -C "$WORKSPACE" config user.email 'blackpepper-e2e@example.invalid'
git -C "$WORKSPACE" add README.md
git -C "$WORKSPACE" commit --quiet -m 'test: initialize Zellij namespace fixture'

export HOME="$TEMP_HOME"
export XDG_CONFIG_HOME="$TEST_ROOT/config"
export XDG_STATE_HOME="$TEST_ROOT/state"
export XDG_CACHE_HOME="$TEST_ROOT/cache"
export XDG_DATA_HOME="$E2E_DATA_HOME"
export SHELL='/bin/bash'
export TERM='xterm-256color'
export LANG='C.UTF-8'
export LC_ALL='C.UTF-8'
export TMPDIR="$HOSTILE_TMPDIR"
unset BLACKPEPPER_E2E_ZELLIJ_SOCKET_DIR

# The first client resembles a non-desktop SSH/browser shell: no XDG runtime,
# plus a hostile native Zellij override that Blackpepper must ignore.
tmux -S "$TMUX_SOCKET" new-session -d -s "$TMUX_SESSION" -x 150 -y 46 \
  -c "$WORKSPACE" env -u XDG_RUNTIME_DIR \
  ZELLIJ_SOCKET_DIR="$HOSTILE_SOCKET_A" "$BP_DEV"
wait_for_screen 'workspace' first-workspace 60

for _attempt in $(seq 1 600); do
  ZELLIJ_BIN="$(find "$E2E_DATA_HOME/blackpepper/sidecars/zellij/0.44.3" \
    -type f -name zellij -perm -u+x -print -quit 2>/dev/null || true)"
  [ -n "$ZELLIJ_BIN" ] && break
  sleep 0.1
done
[ -n "$ZELLIJ_BIN" ] || fail_e2e 'managed Zellij 0.44.3 was not installed'
[ "$($ZELLIJ_BIN --version)" = 'zellij 0.44.3' ] ||
  fail_e2e 'managed Zellij version is not exact'

BACKEND_SESSION="$(session_from_registry)"
valid_backend_session "$BACKEND_SESSION" ||
  fail_e2e "registry returned invalid backend session: $BACKEND_SESSION"
assert_single_canonical_namespace first-startup 150

send_enter
wait_for_terminal_mode first-attached 30
dismiss_zellij_popups
run_shell_command "export BP_NAMESPACE_TOKEN='$NAMESPACE_TOKEN'; printf '\\nBP_NAMESPACE_FIRST:%s\\n' \"\$BP_NAMESPACE_TOKEN\""
wait_for_screen "BP_NAMESPACE_FIRST:$NAMESPACE_TOKEN" first-shell-token 15
run_tui_command ':quit'
wait_for_session_exit 15
wait_for_canonical_client_count 0
assert_single_canonical_namespace first-detach

# Relaunch the same immutable client and registry from a desktop-like shell.
# Both XDG_RUNTIME_DIR and the hostile native socket root now disagree with the
# first launch, yet the one persistent shell must remain canonical.
tmux -S "$TMUX_SOCKET" new-session -d -s "$TMUX_SESSION" -x 150 -y 46 \
  -c "$WORKSPACE" env XDG_RUNTIME_DIR="$SECOND_RUNTIME" \
  ZELLIJ_SOCKET_DIR="$HOSTILE_SOCKET_B" "$BP_DEV"
wait_for_screen 'workspace' second-workspace 60
assert_single_canonical_namespace second-startup
send_enter
wait_for_terminal_mode second-attached 30
dismiss_zellij_popups
run_shell_command "printf '\\nBP_NAMESPACE_SECOND:%s\\n' \"\$BP_NAMESPACE_TOKEN\""
wait_for_screen "BP_NAMESPACE_SECOND:$NAMESPACE_TOKEN" persistent-shell-token 15
wait_for_canonical_client_count 1
assert_single_canonical_namespace second-attach

run_tui_command ':quit'
wait_for_session_exit 15
wait_for_canonical_client_count 0
assert_single_canonical_namespace final-detach

printf 'PASS: one Zellij session and shell survived conflicting XDG/native socket environments (%s).\n' "$BUILD_ID"
