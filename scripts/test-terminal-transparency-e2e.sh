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
ZELLIJ_VERSION="$(python3 "$ROOT/scripts/fixtures/zellij_runtime.py" version)"

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

# Zellij's Unix socket has a 107-byte kernel limit, including its own
# contract/session suffix. Keep the isolated development socket root short.
TEST_ROOT="$(mktemp -d /tmp/bpt.XXXXXX)"
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
ZELLIJ_CACHE_ROOT="$E2E_DATA_HOME/blackpepper/sidecars/zellij/$ZELLIJ_VERSION"
SIZE_LOG="$ARTIFACTS/pty-sizes.log"
RAW_OUTPUT="$ARTIFACTS/outer-output.bin"

wait_for_raw_bytes() {
  local needle_hex="$1"
  local label="$2"
  for _attempt in $(seq 1 100); do
    python3 - "$RAW_OUTPUT" "$needle_hex" <<'PY' && return
import pathlib
import sys

data = pathlib.Path(sys.argv[1]).read_bytes()
needle = bytes.fromhex(sys.argv[2])
raise SystemExit(0 if needle in data else 1)
PY
    sleep 0.1
  done
  fail_e2e "outer terminal did not receive $label"
}

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
tmux_e2e pipe-pane -o -t "$TMUX_SESSION:0.0" "exec cat > '$RAW_OUTPUT'"
tmux_e2e set-option -g set-clipboard on
tmux_e2e set-option -as terminal-features ',xterm-256color:clipboard'
tmux_e2e set-option -g monitor-bell on
wait_for_screen 'bp  blackpepper' startup 60
wait_for_screen 'workspace' workspace 60

for _attempt in $(seq 1 600); do
  ZELLIJ_BIN="$(find "$ZELLIJ_CACHE_ROOT" \
    -type f -name zellij -perm -u+x -print -quit 2>/dev/null || true)"
  [ -n "$ZELLIJ_BIN" ] && break
  sleep 0.1
done
[ -n "$ZELLIJ_BIN" ] || fail_e2e "managed Zellij $ZELLIJ_VERSION was not installed"

send_enter
wait_for_terminal_mode attached 20
assert_screen_lacks 'HOSTS' terminal-hides-hosts
assert_screen_lacks 'PORTS' terminal-hides-ports
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

run_shell_command 'bell'
wait_for_screen 'BP_BELL_SENT' bell-source 10
BELL_FORWARDED=''
for _attempt in $(seq 1 100); do
  BELL_FORWARDED="$(tmux_e2e display-message -p '#{window_bell_flag}')"
  [ "$BELL_FORWARDED" = '1' ] && break
  sleep 0.1
done
[ "$BELL_FORWARDED" = '1' ] || fail_e2e 'outer terminal did not receive BEL'

if [ "${BLACKPEPPER_TERMINAL_E2E_EXPECT_NOTIFICATIONS:-0}" = '1' ]; then
  run_shell_command 'notify'
  wait_for_screen 'BP_OSC9_SENT' osc9-source 10
  wait_for_raw_bytes \
    '1b5d393b42505f4e4f54494649434154494f4e5f45324507' 'OSC 9'

  # The patched Zellij client asks its immediate terminal for focus events.
  # Blackpepper must mirror that mode before an outer CSI O can exist.
  wait_for_raw_bytes '1b5b3f3130303468' 'focus reporting enable'

  run_shell_command 'focus'
  wait_for_screen 'BP_FOCUS_READY' focus-ready 10
  tmux_e2e send-keys -t "$TMUX_SESSION:0.0" -H 1b 5b 4f
  wait_for_screen 'BP_FOCUS_INPUT:1b5b4f' focus-out-delivery 10
fi

run_shell_command 'quit'
wait_for_screen 'BP_TERMINAL_DONE' fixture-done 10
run_tui_command ':quit'
wait_for_session_exit 15

if [ "${BLACKPEPPER_TERMINAL_E2E_EXPECT_NOTIFICATIONS:-0}" = '1' ]; then
  wait_for_raw_bytes '1b5b3f313030346c' 'focus reporting cleanup'
fi

printf 'PASS: native Zellij scroll/search, resize, clipboard, BEL, and optional notification/focus handoff\n'
