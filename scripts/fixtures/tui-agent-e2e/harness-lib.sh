#!/usr/bin/env bash

# PTY/TUI lifecycle helpers. The caller provides all uppercase path globals.

LAST_CAPTURE=''
BLACKPEPPER_MANAGE_MARKER=' MANAGE '
BLACKPEPPER_TERMINAL_ANCHOR='bp  blackpepper'

cleanup_agent_e2e() {
  local status=$?
  set +e
  if [ -n "${ZELLIJ_BIN:-}" ] && [ -x "$ZELLIJ_BIN" ]; then
    ZELLIJ_SOCKET_DIR="$ZELLIJ_SOCKET_ROOT" "$ZELLIJ_BIN" kill-all-sessions -y \
      >/dev/null 2>&1
  fi
  if tmux -S "$TMUX_SOCKET" has-session -t "$TMUX_SESSION" 2>/dev/null; then
    tmux -S "$TMUX_SOCKET" kill-server >/dev/null 2>&1
  fi
  if [ "$status" -ne 0 ] && [ "${BLACKPEPPER_AGENT_E2E_KEEP:-0}" = 1 ]; then
    printf 'Preserved failed agent E2E: %s\n' "$TEST_ROOT" >&2
  else
    case "$TEST_ROOT" in
      "${TMPDIR:-/tmp}"/blackpepper-agent-e2e.*) rm -rf -- "$TEST_ROOT" ;;
      *) printf 'Refusing to clean unexpected test root: %s\n' "$TEST_ROOT" >&2 ;;
    esac
  fi
  exit "$status"
}

capture_screen() {
  local label="${1:-screen}"
  LAST_CAPTURE="$ARTIFACTS/$label.txt"
  if tmux -S "$TMUX_SOCKET" has-session -t "$TMUX_SESSION" 2>/dev/null; then
    tmux -S "$TMUX_SOCKET" capture-pane -p -J -t "$TMUX_SESSION:0.0" > "$LAST_CAPTURE"
  else
    printf '%s\n' '[Blackpepper TUI is not running]' > "$LAST_CAPTURE"
  fi
  cat "$LAST_CAPTURE"
}

fail_agent_e2e() {
  local message="$1"
  printf 'FAIL: %s\n' "$message" >&2
  if [ -n "$LAST_CAPTURE" ] && [ -f "$LAST_CAPTURE" ]; then
    printf '%s\n' '----- exact TUI capture -----' >&2
    sed -n '1,140p' "$LAST_CAPTURE" >&2
    printf '%s\n' '----- end capture -----' >&2
  fi
  printf 'Artifacts: %s\n' "$ARTIFACTS" >&2
  exit 1
}

wait_for_screen() {
  local needle="$1" label="$2" seconds="${3:-20}"
  for _attempt in $(seq 1 $((seconds * 10))); do
    capture_screen "$label" >/dev/null
    grep -Fq -- "$needle" "$LAST_CAPTURE" && return 0
    tmux -S "$TMUX_SOCKET" has-session -t "$TMUX_SESSION" 2>/dev/null ||
      fail_agent_e2e "TUI exited while waiting for: $needle"
    sleep 0.1
  done
  fail_agent_e2e "timed out waiting for screen text: $needle"
}

wait_for_status() {
  local marker="$1" label="$2" seconds="${3:-20}"
  for _attempt in $(seq 1 $((seconds * 10))); do
    capture_screen "$label" >/dev/null
    grep -Fq -- "$marker" "$LAST_CAPTURE" && return 0
    sleep 0.1
  done
  fail_agent_e2e "timed out waiting for public workspace status: $marker"
}

assert_screen_has() {
  local needle="$1" label="$2"
  capture_screen "$label" >/dev/null
  grep -Fq -- "$needle" "$LAST_CAPTURE" ||
    fail_agent_e2e "screen did not contain: $needle"
}

assert_screen_lacks() {
  local needle="$1" label="$2"
  capture_screen "$label" >/dev/null
  if grep -Fq -- "$needle" "$LAST_CAPTURE"; then
    fail_agent_e2e "screen exposed forbidden text: $needle"
  fi
}

send_literal() {
  tmux -S "$TMUX_SOCKET" send-keys -t "$TMUX_SESSION:0.0" -l -- "$1"
}

send_enter() {
  tmux -S "$TMUX_SOCKET" send-keys -t "$TMUX_SESSION:0.0" Enter
}

send_escape() {
  tmux -S "$TMUX_SOCKET" send-keys -t "$TMUX_SESSION:0.0" Escape
}

send_hex() {
  tmux -S "$TMUX_SOCKET" send-keys -t "$TMUX_SESSION:0.0" -H "$1"
}

capture_is_manage() {
  grep -Fq -- "$BLACKPEPPER_MANAGE_MARKER" "$LAST_CAPTURE"
}

capture_is_terminal() {
  grep -Fq -- "$BLACKPEPPER_TERMINAL_ANCHOR" "$LAST_CAPTURE" &&
    ! grep -Fq -- "$BLACKPEPPER_MANAGE_MARKER" "$LAST_CAPTURE" &&
    ! grep -Fq -- ' AUTHENTICATE ' "$LAST_CAPTURE"
}

wait_for_terminal_mode() {
  local label="$1" seconds="${2:-20}"
  for _attempt in $(seq 1 $((seconds * 10))); do
    capture_screen "$label" >/dev/null
    capture_is_terminal && return 0
    tmux -S "$TMUX_SOCKET" has-session -t "$TMUX_SESSION" 2>/dev/null ||
      fail_agent_e2e 'TUI exited while waiting for the Blackpepper terminal status row'
    sleep 0.1
  done
  fail_agent_e2e 'timed out waiting for the Blackpepper terminal status row'
}

ensure_manage() {
  capture_screen mode-check >/dev/null
  capture_is_manage && return 0
  send_hex 1d
  wait_for_screen "$BLACKPEPPER_MANAGE_MARKER" manage-mode 5
}

ensure_work() {
  capture_screen mode-check >/dev/null
  capture_is_terminal && return 0
  send_hex 1d
  wait_for_terminal_mode terminal-mode 5
}

run_tui_command() {
  ensure_manage
  send_literal "$1"
  send_enter
}

dismiss_zellij_popups() {
  for _attempt in 1 2 3; do
    capture_screen popup-check >/dev/null
    if grep -Fq 'First Run Setup Wizard' "$LAST_CAPTURE"; then
      send_enter
      sleep 0.5
    elif grep -Fq 'Release Notes ' "$LAST_CAPTURE"; then
      send_escape
      sleep 0.5
    else
      return 0
    fi
  done
  fail_agent_e2e 'Zellij popup could not be dismissed'
}

start_client() {
  tmux -S "$TMUX_SOCKET" new-session -d -s "$TMUX_SESSION" -x 170 -y 46 \
    -c "$WORKSPACE" "$BP_BINARY"
  wait_for_screen 'bp  blackpepper' startup 60
  wait_for_screen 'HOSTS' startup-hosts 30
  wait_for_screen 'SESSION' startup-session 30
  wait_for_screen 'PORTS' startup-ports 30
  wait_for_screen 'fixture' startup-workspace 30
}

attach_workspace() {
  capture_screen attach-check >/dev/null
  if ! capture_is_terminal; then
    send_enter
    wait_for_terminal_mode attached 30
  fi
  dismiss_zellij_popups
}

stop_client() {
  run_tui_command ':quit'
  for _attempt in $(seq 1 100); do
    tmux -S "$TMUX_SOCKET" has-session -t "$TMUX_SESSION" 2>/dev/null || return 0
    sleep 0.1
  done
  fail_agent_e2e 'Blackpepper did not exit after :quit'
}

state_tool() {
  python3 "$FIXTURES/agent-e2e-state.py" --root "$PROVIDER_ROOT" "$@"
}

wait_for_asset_absent() {
  local path="$1"
  [ -z "$path" ] && return 0
  for _attempt in $(seq 1 200); do
    [ ! -e "$path" ] && return 0
    sleep 0.1
  done
  fail_agent_e2e "managed integration asset was not cleaned: $path"
}
