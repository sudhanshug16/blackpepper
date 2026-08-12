#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)"
FIXTURES="$ROOT/scripts/fixtures/tui-worktrunk-e2e"
# shellcheck source=scripts/fixtures/tui-worktrunk-e2e/harness-lib.sh
source "$FIXTURES/harness-lib.sh"

for requirement in git python3 timeout tmux; do
  command -v "$requirement" >/dev/null 2>&1 || {
    printf 'FAIL: required command is unavailable: %s\n' "$requirement" >&2
    exit 1
  }
done
[ "$(uname -s)" = Linux ] || {
  printf '%s\n' 'FAIL: guarded Worktrunk process-group acceptance currently requires Linux.' >&2
  exit 1
}

BP_CANDIDATE="${BLACKPEPPER_WORKTRUNK_E2E_BP:-$(command -v bp-dev 2>/dev/null || true)}"
if [ -z "$BP_CANDIDATE" ] || [ ! -x "$BP_CANDIDATE" ]; then
  printf '%s\n' 'FAIL: bp-dev was not found; run scripts/setup.sh first.' >&2
  exit 1
fi
if [ -z "${BLACKPEPPER_WORKTRUNK_E2E_BP:-}" ]; then
  launcher_dir="$(CDPATH='' cd -P -- "$(dirname "$BP_CANDIDATE")" && pwd)"
  BP_CANDIDATE="$launcher_dir/.blackpepper-dev/current/bp-dev"
fi
BP_BIN="$(readlink -f -- "$BP_CANDIDATE")"
BP_HOST="${BLACKPEPPER_WORKTRUNK_E2E_BP_HOST:-$(dirname "$BP_BIN")/bp-host}"
[ -x "$BP_BIN" ] || { printf 'FAIL: Blackpepper binary is invalid: %s\n' "$BP_BIN" >&2; exit 1; }
[ -x "$BP_HOST" ] || { printf 'FAIL: matching bp-host is invalid: %s\n' "$BP_HOST" >&2; exit 1; }
BP_BUILD="$($BP_BIN --version | sed 's/^blackpepper //')"
HOST_BUILD="$($BP_HOST --version | sed 's/^bp-host //')"
[ "$BP_BUILD" = "$HOST_BUILD" ] || {
  printf 'FAIL: bp (%s) and bp-host (%s) builds differ.\n' "$BP_BUILD" "$HOST_BUILD" >&2
  exit 1
}
case "$BP_BUILD" in
  *-dev.*) ;;
  *) printf 'FAIL: expected a development build, got: %s\n' "$BP_BUILD" >&2; exit 1 ;;
esac

# Zellij's Unix socket has a 107-byte kernel limit, including its own
# contract/session suffix, so keep this isolated root deliberately short.
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/bp-wt.XXXXXX")"
ARTIFACTS="$TEST_ROOT/artifacts"
PRIMARY="$TEST_ROOT/primary"
OPEN_PATH="$TEST_ROOT/worktrees/open-me"
SETUP_FAILED_PATH="$TEST_ROOT/worktrees/setup-fails"
LOCK_REPO="$TEST_ROOT/lock-repo"
LOCK_MARKER="$TEST_ROOT/lock.marker"
LOCK_RELEASE="$TEST_ROOT/lock.release"
LOST_SURVIVOR="$TEST_ROOT/lost-survivor"
LOST_TARGET="$TEST_ROOT/lost-target"
LOST_MARKER="$TEST_ROOT/lost.marker"
TMUX_SOCKET="$TEST_ROOT/tmux.sock"
TMUX_SESSION='bp-worktrunk-e2e'
ZELLIJ_SOCKET_ROOT="$TEST_ROOT/zellij-sockets"
ZELLIJ_BIN=''
E2E_DATA_HOME="${BLACKPEPPER_WORKTRUNK_E2E_DATA_HOME:-$ROOT/target/tui-local-e2e-data}"

trap cleanup_worktrunk_e2e EXIT HUP INT TERM
install -d -m 0700 \
  "$ARTIFACTS" "$PRIMARY/.config" "$TEST_ROOT/home" "$TEST_ROOT/config/worktrunk" \
  "$TEST_ROOT/state" "$TEST_ROOT/run" "$TEST_ROOT/cache" "$ZELLIJ_SOCKET_ROOT" \
  "$E2E_DATA_HOME"

HOOK="$FIXTURES/controlled-hook.sh"
printf 'worktree-path = "%s/worktrees/{{ branch | sanitize }}"\n' "$TEST_ROOT" \
  > "$TEST_ROOT/config/worktrunk/config.toml"
printf '[pre-start]\nfixture = "%s setup {{ branch }} %s"\n' "$HOOK" "$TEST_ROOT" \
  > "$PRIMARY/.config/wt.toml"
printf '%s\n' '# Blackpepper Worktrunk acceptance fixture' > "$PRIMARY/README.md"
git -C "$PRIMARY" init --initial-branch=main --quiet
git -C "$PRIMARY" config user.name 'Blackpepper E2E'
git -C "$PRIMARY" config user.email 'blackpepper-e2e@example.invalid'
git -C "$PRIMARY" add README.md .config/wt.toml
git -C "$PRIMARY" commit --quiet -m 'test: initialize Worktrunk fixture'
git -C "$PRIMARY" branch open-me

export HOME="$TEST_ROOT/home"
export XDG_CONFIG_HOME="$TEST_ROOT/config"
export XDG_STATE_HOME="$TEST_ROOT/state"
export XDG_RUNTIME_DIR="$TEST_ROOT/run"
export XDG_CACHE_HOME="$TEST_ROOT/cache"
export XDG_DATA_HOME="$E2E_DATA_HOME"
export BLACKPEPPER_E2E_ZELLIJ_SOCKET_DIR="$ZELLIJ_SOCKET_ROOT"
unset ZELLIJ_SOCKET_DIR
export SHELL='/bin/bash'
export TERM='xterm-256color'
export LANG='C.UTF-8'
export LC_ALL='C.UTF-8'

tmux -S "$TMUX_SOCKET" new-session -d -s "$TMUX_SESSION" -x 180 -y 52 \
  -c "$PRIMARY" "$BP_BIN"
wait_for_worktrunk_screen ' MANAGE ' startup 60
wait_for_worktrunk_screen 'primary' primary-startup 60
for _attempt in $(seq 1 200); do
  ZELLIJ_BIN="$(find "$E2E_DATA_HOME/blackpepper/sidecars/zellij/0.44.3" \
    -type f -name zellij -perm -u+x -print -quit 2>/dev/null || true)"
  [ -n "$ZELLIJ_BIN" ] && break
  sleep 0.1
done
[ -x "$ZELLIJ_BIN" ] || fail_worktrunk_e2e 'managed Zellij 0.44.3 was not installed'
[ "$($ZELLIJ_BIN --version)" = 'zellij 0.44.3' ] ||
  fail_worktrunk_e2e 'managed Zellij version is not exact'

run_worktrunk_tui_command ':worktree open open-me'
wait_for_worktrunk_screen 'WORKTRUNK MUTATION' open-review 60
assert_worktrunk_screen_has 'switch open-me' open-selector
assert_worktrunk_screen_has 'PROJECT COMMAND' open-project-command
assert_worktrunk_screen_lacks '--force' open-no-force
run_worktrunk_tui_command ':approve'
wait_for_worktrunk_screen "Registered worktree $OPEN_PATH with a persistent shell." open-done 60
wait_for_worktrunk_file "$TEST_ROOT/setup-open-me.ran" 15
[ -d "$OPEN_PATH" ] || fail_worktrunk_e2e 'branch open did not create its real worktree'
dismiss_worktrunk_zellij_popups

run_worktrunk_tui_command ':workspace switch primary'
wait_for_worktrunk_screen ' WORK ' primary-again 30
dismiss_worktrunk_zellij_popups
run_worktrunk_tui_command ':worktree create setup-fails --base main'
wait_for_worktrunk_screen 'WORKTRUNK MUTATION' setup-review 30
assert_worktrunk_screen_has '--create setup-fails' setup-create-command
run_worktrunk_tui_command ':approve'
wait_for_worktrunk_screen 'exists but setup failed:' setup-visible 60
wait_for_worktrunk_file "$TEST_ROOT/setup-setup-fails.ran" 15
[ -d "$SETUP_FAILED_PATH" ] || fail_worktrunk_e2e 'failed setup hid or deleted its worktree'
assert_worktrunk_screen_has 'setup-fails setup-fail' setup-sidebar
run_worktrunk_tui_command ':refresh'
wait_for_worktrunk_screen 'Refreshed 0 connected remote host(s)' setup-refresh 20
assert_worktrunk_screen_has 'setup-fails setup-fail' setup-persistent
run_worktrunk_tui_command ':quit'
wait_for_worktrunk_session_exit 20

WT_BIN="$(find "$E2E_DATA_HOME/blackpepper/sidecars/worktrunk/0.72.0" \
  -type f -name wt -perm -u+x -print -quit 2>/dev/null || true)"
[ -x "$WT_BIN" ] || fail_worktrunk_e2e 'managed Worktrunk 0.72.0 was not installed'
[ "$($WT_BIN --version)" = 'wt v0.72.0' ] ||
  fail_worktrunk_e2e 'managed Worktrunk version is not exact'

install -d -m 0700 "$LOCK_REPO/.config" "$LOST_SURVIVOR/.config"
printf '[pre-start]\nhold = "%s hold %s %s"\n' "$HOOK" "$LOCK_MARKER" "$LOCK_RELEASE" \
  > "$LOCK_REPO/.config/wt.toml"
printf '%s\n' lock > "$LOCK_REPO/README.md"
git -C "$LOCK_REPO" init --initial-branch=main --quiet
git -C "$LOCK_REPO" config user.name 'Blackpepper E2E'
git -C "$LOCK_REPO" config user.email 'blackpepper-e2e@example.invalid'
git -C "$LOCK_REPO" add README.md .config/wt.toml
git -C "$LOCK_REPO" commit --quiet -m 'test: initialize lock fixture'

printf '[pre-remove]\nhold = "%s remove-gate %s %s %s"\n' \
  "$HOOK" "$LOST_MARKER" "$TEST_ROOT/lost.release" "$TEST_ROOT/lost.count" \
  > "$LOST_SURVIVOR/.config/wt.toml"
printf '%s\n' lost > "$LOST_SURVIVOR/README.md"
git -C "$LOST_SURVIVOR" init --initial-branch=main --quiet
git -C "$LOST_SURVIVOR" config user.name 'Blackpepper E2E'
git -C "$LOST_SURVIVOR" config user.email 'blackpepper-e2e@example.invalid'
git -C "$LOST_SURVIVOR" add README.md .config/wt.toml
git -C "$LOST_SURVIVOR" commit --quiet -m 'test: initialize removal fixture'
git -C "$LOST_SURVIVOR" worktree add --quiet -b lost-remove "$LOST_TARGET"

timeout 120 python3 "$FIXTURES/host-driver.py" \
  "$BP_HOST" "$WT_BIN" "$SETUP_FAILED_PATH" "$LOCK_REPO" "$LOCK_MARKER" \
  "$LOCK_RELEASE" "$LOST_SURVIVOR" "$LOST_TARGET" "$LOST_MARKER"

printf 'PASS: real Worktrunk 0.72.0 branch open and setup-failed registration (%s)\n' "$BP_BUILD"
printf '%s\n' 'PASS: cross-process mutation refusal and lost-response removal reconciliation without retry'
