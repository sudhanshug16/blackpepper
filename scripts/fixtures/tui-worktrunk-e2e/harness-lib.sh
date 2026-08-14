#!/usr/bin/env bash

# Small tmux driver shared by the real Worktrunk edge acceptance. The caller
# owns every path and PID so a failed run can be retained without hidden state.

LAST_CAPTURE=''
BLACKPEPPER_MANAGE_MARKER=' MANAGE '
BLACKPEPPER_TERMINAL_ANCHOR='bp  '

fail_worktrunk_e2e() {
  local message="$1"
  printf 'FAIL: %s\n' "$message" >&2
  if [ -n "$LAST_CAPTURE" ] && [ -f "$LAST_CAPTURE" ]; then
    printf '%s\n' '--- exact last Blackpepper screen ---' >&2
    sed -n '1,160p' "$LAST_CAPTURE" >&2
    printf '%s\n' '--- end screen ---' >&2
  fi
  printf 'Artifacts: %s\n' "$ARTIFACTS" >&2
  exit 1
}

cleanup_worktrunk_e2e() {
  local status=$?
  set +e
  if [ -n "${ZELLIJ_BIN:-}" ] && [ -x "$ZELLIJ_BIN" ]; then
    ZELLIJ_SOCKET_DIR="$ZELLIJ_SOCKET_ROOT" \
      "$ZELLIJ_BIN" kill-all-sessions --yes >/dev/null 2>&1 || true
  fi
  tmux -S "$TMUX_SOCKET" kill-server >/dev/null 2>&1 || true
  if [ "${BLACKPEPPER_WORKTRUNK_E2E_KEEP:-0}" = 1 ]; then
    printf 'Worktrunk E2E artifacts retained at %s\n' "$TEST_ROOT" >&2
  else
    case "$TEST_ROOT" in
      /tmp/bp-wt.*)
        find "$TEST_ROOT" -depth -delete >/dev/null 2>&1 || true
        ;;
      *) printf 'Refusing to clean unexpected test root: %s\n' "$TEST_ROOT" >&2 ;;
    esac
  fi
  exit "$status"
}

tmux_worktrunk() {
  tmux -S "$TMUX_SOCKET" "$@"
}

session_is_live() {
  tmux_worktrunk has-session -t "$TMUX_SESSION" 2>/dev/null
}

capture_worktrunk_screen() {
  local label="$1"
  LAST_CAPTURE="$ARTIFACTS/${label}.txt"
  if session_is_live; then
    tmux_worktrunk capture-pane -p -J -t "$TMUX_SESSION:0.0" > "$LAST_CAPTURE"
  else
    printf '%s\n' '[tmux session is no longer running]' > "$LAST_CAPTURE"
  fi
}

wait_for_worktrunk_screen() {
  local needle="$1" label="$2" timeout_seconds="${3:-20}"
  local deadline=$((SECONDS + timeout_seconds))
  while [ "$SECONDS" -lt "$deadline" ]; do
    capture_worktrunk_screen "$label"
    grep -Fq -- "$needle" "$LAST_CAPTURE" && return 0
    session_is_live || fail_worktrunk_e2e "TUI exited while waiting for: $needle"
    sleep 0.1
  done
  fail_worktrunk_e2e "timed out after ${timeout_seconds}s waiting for: $needle"
}

assert_worktrunk_screen_has() {
  local needle="$1" label="$2"
  capture_worktrunk_screen "$label"
  grep -Fq -- "$needle" "$LAST_CAPTURE" ||
    fail_worktrunk_e2e "screen did not contain: $needle"
}

assert_worktrunk_screen_lacks() {
  local needle="$1" label="$2"
  capture_worktrunk_screen "$label"
  if grep -Fq -- "$needle" "$LAST_CAPTURE"; then
    fail_worktrunk_e2e "screen unexpectedly contained: $needle"
  fi
}

worktrunk_capture_is_manage() {
  grep -Fq -- "$BLACKPEPPER_MANAGE_MARKER" "$LAST_CAPTURE"
}

worktrunk_capture_is_terminal() {
  tail -n 1 "$LAST_CAPTURE" | grep -Fq -- "$BLACKPEPPER_TERMINAL_ANCHOR" &&
    ! grep -Fq -- "$BLACKPEPPER_MANAGE_MARKER" "$LAST_CAPTURE" &&
    ! grep -Fq -- ' AUTHENTICATE ' "$LAST_CAPTURE"
}

wait_for_worktrunk_terminal_mode() {
  local label="$1" timeout_seconds="${2:-20}"
  local deadline=$((SECONDS + timeout_seconds))
  while [ "$SECONDS" -lt "$deadline" ]; do
    capture_worktrunk_screen "$label"
    worktrunk_capture_is_terminal && return 0
    session_is_live ||
      fail_worktrunk_e2e 'TUI exited while waiting for the Blackpepper terminal status row'
    sleep 0.1
  done
  fail_worktrunk_e2e 'timed out waiting for the Blackpepper terminal status row'
}

send_worktrunk_literal() {
  tmux_worktrunk send-keys -t "$TMUX_SESSION:0.0" -l -- "$1"
}

send_worktrunk_hex() {
  tmux_worktrunk send-keys -t "$TMUX_SESSION:0.0" -H "$1"
}

ensure_worktrunk_manage_mode() {
  capture_worktrunk_screen mode-check
  if worktrunk_capture_is_manage; then
    return 0
  fi
  send_worktrunk_hex 1d
  wait_for_worktrunk_screen "$BLACKPEPPER_MANAGE_MARKER" manage-mode 5
}

run_worktrunk_tui_command() {
  ensure_worktrunk_manage_mode
  send_worktrunk_literal "$1"
  tmux_worktrunk send-keys -t "$TMUX_SESSION:0.0" Enter
}

dismiss_worktrunk_zellij_popups() {
  local iteration=0
  while [ "$iteration" -lt 3 ]; do
    capture_worktrunk_screen popup-check
    if grep -Fq 'First Run Setup Wizard' "$LAST_CAPTURE"; then
      tmux_worktrunk send-keys -t "$TMUX_SESSION:0.0" Enter
      sleep 0.5
    elif grep -Fq 'Release Notes ' "$LAST_CAPTURE"; then
      tmux_worktrunk send-keys -t "$TMUX_SESSION:0.0" Escape
      sleep 0.5
    else
      return 0
    fi
    iteration=$((iteration + 1))
  done
  fail_worktrunk_e2e 'Zellij first-run popup could not be dismissed'
}

wait_for_worktrunk_session_exit() {
  local timeout_seconds="${1:-15}"
  local deadline=$((SECONDS + timeout_seconds))
  while [ "$SECONDS" -lt "$deadline" ]; do
    session_is_live || return 0
    sleep 0.1
  done
  capture_worktrunk_screen quit-timeout
  fail_worktrunk_e2e "TUI did not exit within ${timeout_seconds}s"
}

wait_for_worktrunk_file() {
  local path="$1" timeout_seconds="${2:-15}"
  local deadline=$((SECONDS + timeout_seconds))
  while [ "$SECONDS" -lt "$deadline" ]; do
    [ -s "$path" ] && return 0
    sleep 0.1
  done
  fail_worktrunk_e2e "timed out waiting for fixture marker: $path"
}
