#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)"
FIXTURES="$ROOT/scripts/fixtures/tui-local-e2e"
TERMINAL_FIXTURE="$ROOT/scripts/fixtures/terminal-e2e/terminal_fixture.py"
# shellcheck source=scripts/fixtures/tui-local-e2e/harness-lib.sh
source "$FIXTURES/harness-lib.sh"

for requirement in tmux python3 timeout; do
  command -v "$requirement" >/dev/null 2>&1 || {
    printf 'FAIL: required command is unavailable: %s\n' "$requirement" >&2
    exit 1
  }
done

BP_CANDIDATE="${BLACKPEPPER_TERMINAL_E2E_BP:-$(command -v bp-dev 2>/dev/null || true)}"
if [ -z "$BP_CANDIDATE" ] || [ ! -x "$BP_CANDIDATE" ]; then
  printf '%s\n' 'FAIL: bp-dev was not found; run scripts/setup.sh first.' >&2
  exit 1
fi
if [ -z "${BLACKPEPPER_TERMINAL_E2E_BP:-}" ]; then
  launcher_dir="$(CDPATH='' cd -P -- "$(dirname "$BP_CANDIDATE")" && pwd)"
  BP_CANDIDATE="$launcher_dir/.blackpepper-dev/current/bp-dev"
fi
BP_DEV="$(readlink -f -- "$BP_CANDIDATE")"
[ -x "$BP_DEV" ] || {
  printf 'FAIL: installed bp-dev executable is invalid: %s\n' "$BP_DEV" >&2
  exit 1
}
BP_VERSION="$($BP_DEV --version 2>&1)"
case "$BP_VERSION" in
  'blackpepper '*'-dev.'*) ;;
  *) printf 'FAIL: expected a development build, got: %s\n' "$BP_VERSION" >&2; exit 1 ;;
esac

TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/blackpepper-terminal-e2e.XXXXXX")"
ARTIFACTS="$TEST_ROOT/artifacts"
PRIMARY="$TEST_ROOT/workspace"
TEMP_HOME="$TEST_ROOT/home"
TMUX_SOCKET="$TEST_ROOT/tmux.sock"
TMUX_SESSION='bp-terminal-e2e'
ZELLIJ_BIN=''
ZELLIJ_SOCKET_ROOT="$TEST_ROOT/z"
LISTENER_PID=''
SECOND_TMUX_SOCKET=''
E2E_DATA_HOME="${BLACKPEPPER_TUI_E2E_DATA_HOME:-$ROOT/target/tui-local-e2e-data}"
SIZE_LOG="$ARTIFACTS/pty-sizes.log"

case "$E2E_DATA_HOME" in
  /*) ;;
  *) fail_e2e "BLACKPEPPER_TUI_E2E_DATA_HOME must be absolute: $E2E_DATA_HOME" ;;
esac
trap cleanup_e2e EXIT HUP INT TERM

install -d -m 0700 "$ARTIFACTS" "$PRIMARY" "$TEMP_HOME" \
  "$TEST_ROOT/config" "$TEST_ROOT/state" "$TEST_ROOT/run" \
  "$TEST_ROOT/cache" "$ZELLIJ_SOCKET_ROOT" "$E2E_DATA_HOME"

export HOME="$TEMP_HOME"
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
unset DISPLAY WAYLAND_DISPLAY

tmux -S "$TMUX_SOCKET" new-session -d -s "$TMUX_SESSION" -x 150 -y 46 \
  -c "$PRIMARY" "$BP_DEV"
tmux_e2e set-option -g set-clipboard on
tmux_e2e set-option -as terminal-features ',xterm-256color:clipboard'
wait_for_screen 'Blackpepper' startup 60
wait_for_screen 'workspace' workspace 60

for _attempt in $(seq 1 600); do
  ZELLIJ_BIN="$(find "$E2E_DATA_HOME/blackpepper/sidecars/zellij/0.44.3" \
    -type f -name zellij -perm -u+x -print -quit 2>/dev/null || true)"
  [ -n "$ZELLIJ_BIN" ] && break
  sleep 0.1
done
[ -n "$ZELLIJ_BIN" ] || fail_e2e 'managed Zellij 0.44.3 was not installed'

send_enter
wait_for_screen ' WORK ' attached 20
assert_screen_lacks 'Hosts / Workspaces' work-hides-sidebar
assert_screen_lacks ' Ports ' work-hides-ports
dismiss_zellij_popups
run_shell_command "BP_TERMINAL_SIZE_LOG='$SIZE_LOG' python3 '$TERMINAL_FIXTURE'"
wait_for_screen 'BP_TERMINAL_READY' fixture-ready 15
BEFORE_SIZE="$(tail -n 1 "$SIZE_LOG" 2>/dev/null || true)"
[ -n "$BEFORE_SIZE" ] || fail_e2e 'fixture did not report its initial PTY dimensions'

send_hex 13
wait_for_screen 'SCROLL' native-scroll 10
send_literal 's'
wait_for_screen 'ENTERING SEARCH TERM' native-enter-search 10
send_literal 'BP_SCROLL_LINE_007'
send_enter
wait_for_screen 'BP_SCROLL_LINE_007' native-search-result 10
assert_screen_has 'SEARCH' native-search-mode

# Zellij's native Search binding uses Ctrl-C to return to Normal at the live
# bottom without forwarding an interrupt into the fixture process.
send_hex 03
wait_for_screen_absent 'SEARCHING' native-search-exit 5
tmux_e2e resize-window -t "$TMUX_SESSION:0" -x 112 -y 32
AFTER_SIZE=''
for _attempt in $(seq 1 100); do
  capture_screen after-resize
  AFTER_SIZE="$(tail -n 1 "$SIZE_LOG" 2>/dev/null || true)"
  [ -n "$AFTER_SIZE" ] && [ "$AFTER_SIZE" != "$BEFORE_SIZE" ] && break
  sleep 0.1
done
if [ -z "$AFTER_SIZE" ] || [ "$AFTER_SIZE" = "$BEFORE_SIZE" ]; then
  fail_e2e "embedded PTY did not resize (remained $BEFORE_SIZE)"
fi

run_shell_command 'copy'
wait_for_screen 'BP_OSC52_SENT' osc52-source 10
wait_for_screen 'Copy sent to your terminal.' osc52-visible-handoff 10
assert_screen_lacks 'Copy failed.' osc52-nonfatal
TMUX_COPY="$(tmux_e2e show-buffer 2>/dev/null || true)"
[ "$TMUX_COPY" = 'blackpepper-osc52-e2e' ] ||
  fail_e2e "outer terminal did not receive normalized OSC 52 (got: $TMUX_COPY)"

run_shell_command 'quit'
wait_for_screen 'BP_TERMINAL_DONE' fixture-done 10
run_tui_command ':quit'
wait_for_session_exit 15

printf 'PASS: focused Work canvas, native Zellij scroll/search, resize, and clipboard handoff\n'
