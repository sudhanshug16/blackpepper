#!/usr/bin/env bash

# Shared assertions for scripts/test-tui-local-e2e.sh. The caller owns all
# lifecycle state and provides TMUX_SOCKET, TMUX_SESSION, and ARTIFACTS.

LAST_CAPTURE=''
# shellcheck source=scripts/fixtures/tui-local-e2e/ui-markers-lib.sh
source "$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/ui-markers-lib.sh"

cleanup_e2e() {
  local status=$?
  # Signal handlers exit through this function; disarm every trap first so
  # cleanup cannot re-enter and accidentally replace the signal status.
  trap - EXIT HUP INT TERM
  set +e
  if [ -n "$ZELLIJ_BIN" ] && [ -x "$ZELLIJ_BIN" ]; then
    ZELLIJ_SOCKET_DIR="$ZELLIJ_SOCKET_ROOT" "$ZELLIJ_BIN" kill-all-sessions -y \
      >/dev/null 2>&1
  fi
  if tmux -S "$TMUX_SOCKET" has-session -t "$TMUX_SESSION" 2>/dev/null; then
    tmux -S "$TMUX_SOCKET" kill-server >/dev/null 2>&1
  fi
  if [ -n "${SECOND_TMUX_SOCKET:-}" ] && tmux -S "$SECOND_TMUX_SOCKET" has-session 2>/dev/null; then
    tmux -S "$SECOND_TMUX_SOCKET" kill-server >/dev/null 2>&1
  fi
  if [ -n "$LISTENER_PID" ] && kill -0 "$LISTENER_PID" 2>/dev/null; then
    kill "$LISTENER_PID" 2>/dev/null
    wait "$LISTENER_PID" 2>/dev/null
  fi
  if [ "$status" -ne 0 ] && [ "${BLACKPEPPER_TUI_E2E_KEEP:-0}" = 1 ]; then
    printf 'Preserved failed run: %s\n' "$TEST_ROOT" >&2
  else
    case "$TEST_ROOT" in
      /tmp/bpl.*|/tmp/bpt.*)
        rm -rf -- "$TEST_ROOT"
        ;;
      *) printf 'Refusing to clean unexpected test root: %s\n' "$TEST_ROOT" >&2 ;;
    esac
  fi
  exit "$status"
}

tmux_e2e() {
  tmux -S "$TMUX_SOCKET" "$@"
}

session_is_live() {
  tmux_e2e has-session -t "$TMUX_SESSION" 2>/dev/null
}

capture_screen() {
  local label="$1"
  LAST_CAPTURE="$ARTIFACTS/${label}.txt"
  if session_is_live; then
    tmux_e2e capture-pane -p -J -t "$TMUX_SESSION:0.0" > "$LAST_CAPTURE"
  else
    printf '%s\n' '[tmux session is no longer running]' > "$LAST_CAPTURE"
  fi
}

fail_e2e() {
  local message="$1"
  printf 'FAIL: %s\n' "$message" >&2
  if [ -n "$LAST_CAPTURE" ] && [ -f "$LAST_CAPTURE" ]; then
    printf '%s\n' '----- exact last TUI capture -----' >&2
    sed -n '1,160p' "$LAST_CAPTURE" >&2
    printf '%s\n' '----- end TUI capture -----' >&2
  fi
  printf 'Artifacts: %s\n' "$ARTIFACTS" >&2
  exit 1
}

wait_for_screen() {
  local needle="$1" label="$2" timeout_seconds="${3:-20}"
  local attempts=$((timeout_seconds * 10))
  local attempt=0
  while [ "$attempt" -lt "$attempts" ]; do
    capture_screen "$label"
    if grep -Fq -- "$needle" "$LAST_CAPTURE"; then
      return 0
    fi
    if ! session_is_live; then
      fail_e2e "TUI exited while waiting for: $needle"
    fi
    sleep 0.1
    attempt=$((attempt + 1))
  done
  fail_e2e "timed out after ${timeout_seconds}s waiting for screen text: $needle"
}

wait_for_screen_absent() {
  local needle="$1" label="$2" timeout_seconds="${3:-20}"
  local attempts=$((timeout_seconds * 10))
  local attempt=0
  while [ "$attempt" -lt "$attempts" ]; do
    capture_screen "$label"
    if ! grep -Fq -- "$needle" "$LAST_CAPTURE"; then
      return 0
    fi
    if ! session_is_live; then
      fail_e2e "TUI exited while waiting for text to disappear: $needle"
    fi
    sleep 0.1
    attempt=$((attempt + 1))
  done
  fail_e2e "timed out after ${timeout_seconds}s waiting for screen text to disappear: $needle"
}

assert_screen_has() {
  local needle="$1" label="$2"
  capture_screen "$label"
  grep -Fq -- "$needle" "$LAST_CAPTURE" ||
    fail_e2e "screen did not contain: $needle"
}

assert_screen_lacks() {
  local needle="$1" label="$2"
  capture_screen "$label"
  if grep -Fq -- "$needle" "$LAST_CAPTURE"; then
    fail_e2e "screen unexpectedly contained: $needle"
  fi
}

send_literal() {
  tmux_e2e send-keys -t "$TMUX_SESSION:0.0" -l -- "$1"
}

send_enter() {
  tmux_e2e send-keys -t "$TMUX_SESSION:0.0" Enter
}

send_escape() {
  tmux_e2e send-keys -t "$TMUX_SESSION:0.0" Escape
}

send_hex() {
  tmux_e2e send-keys -t "$TMUX_SESSION:0.0" -H "$1"
}

ensure_manage_mode() {
  capture_screen mode-check
  if capture_is_manage_mode; then
    return 0
  fi
  send_hex 1d
  wait_for_screen "$BLACKPEPPER_MANAGE_MARKER" mode-manage 5
}

ensure_work_mode() {
  capture_screen mode-check
  if capture_is_terminal_mode; then
    return 0
  fi
  send_hex 1d
  wait_for_terminal_mode mode-terminal 5
}

dismiss_zellij_popups() {
  local iteration=0
  sleep 0.5
  while [ "$iteration" -lt 3 ]; do
    capture_screen zellij-popup-check
    if grep -Fq 'First Run Setup Wizard' "$LAST_CAPTURE"; then
      send_enter
      wait_for_screen_absent 'First Run Setup Wizard' zellij-first-run-accepted 10
    elif grep -Fq 'Release Notes ' "$LAST_CAPTURE"; then
      send_escape
      wait_for_screen_absent 'Release Notes ' zellij-release-notes-dismissed 10
    elif grep -Fq 'About Zellij' "$LAST_CAPTURE"; then
      send_escape
      wait_for_screen_absent 'About Zellij' zellij-about-dismissed 10
    else
      return 0
    fi
    sleep 0.2
    iteration=$((iteration + 1))
  done
  capture_screen zellij-popup-stuck
  fail_e2e 'Zellij first-run popup could not be dismissed'
}

run_tui_command() {
  ensure_manage_mode
  send_literal "$1"
  send_enter
}

run_shell_command() {
  ensure_work_mode
  send_literal "$1"
  send_enter
}

wait_for_file_text() {
  local path="$1" expected="$2" timeout_seconds="${3:-20}"
  local attempts=$((timeout_seconds * 10))
  local attempt=0
  while [ "$attempt" -lt "$attempts" ]; do
    if [ -f "$path" ] && [ "$(tr -d '\r\n' < "$path")" = "$expected" ]; then
      return 0
    fi
    sleep 0.1
    attempt=$((attempt + 1))
  done
  fail_e2e "timed out waiting for $path to contain: $expected"
}

wait_for_file_lines() {
  local path="$1" expected_text="$2" expected_count="$3" timeout_seconds="${4:-20}"
  local attempts=$((timeout_seconds * 10))
  local attempt=0 matching_count=0 total_count=0
  while [ "$attempt" -lt "$attempts" ]; do
    if [ -f "$path" ]; then
      matching_count="$(grep -Fxc -- "$expected_text" "$path" || true)"
      total_count="$(wc -l < "$path" | tr -d '[:space:]')"
      if [ "$matching_count" -eq "$expected_count" ] && [ "$total_count" -eq "$expected_count" ]; then
        return 0
      fi
    fi
    sleep 0.1
    attempt=$((attempt + 1))
  done
  fail_e2e "timed out waiting for $path to contain exactly $expected_count line(s): $expected_text"
}

wait_for_path_absent() {
  local path="$1" timeout_seconds="${2:-20}"
  local attempts=$((timeout_seconds * 10))
  local attempt=0
  while [ "$attempt" -lt "$attempts" ]; do
    [ ! -e "$path" ] && return 0
    sleep 0.1
    attempt=$((attempt + 1))
  done
  fail_e2e "timed out waiting for path removal: $path"
}

wait_for_session_exit() {
  local timeout_seconds="${1:-15}"
  local attempts=$((timeout_seconds * 10))
  local attempt=0
  while [ "$attempt" -lt "$attempts" ]; do
    session_is_live || return 0
    sleep 0.1
    attempt=$((attempt + 1))
  done
  capture_screen quit-timeout
  fail_e2e "TUI did not exit within ${timeout_seconds}s"
}

wait_for_zellij_client_count() {
  local session="$1" expected="$2" timeout_seconds="${3:-15}"
  local attempts=$((timeout_seconds * 10))
  local attempt=0 output count
  while [ "$attempt" -lt "$attempts" ]; do
    output="$(ZELLIJ_SOCKET_DIR="$ZELLIJ_SOCKET_ROOT" \
      "$ZELLIJ_BIN" --session "$session" action list-clients 2>/dev/null || true)"
    count="$(printf '%s\n' "$output" | awk 'NR > 1 && NF { count += 1 } END { print count + 0 }')"
    [ "$count" -eq "$expected" ] && return 0
    sleep 0.1
    attempt=$((attempt + 1))
  done
  capture_screen zellij-client-count-timeout
  fail_e2e "Zellij session $session did not reach $expected controlling client(s)"
}

start_second_zellij_client() {
  tmux -S "$SECOND_TMUX_SOCKET" new-session -d -s bp-second -x 100 -y 30 \
    -c "$PRIMARY" "$SECOND_COMMAND"
}

prepare_backend_session() {
  BACKEND_SESSION="$(ZELLIJ_SOCKET_DIR="$ZELLIJ_SOCKET_ROOT" "$ZELLIJ_BIN" \
    list-sessions --short --no-formatting)"
  case "$BACKEND_SESSION" in
    bp-*) ;;
    *) fail_e2e "expected one Blackpepper Zellij session, got: $BACKEND_SESSION" ;;
  esac
  printf -v SECOND_COMMAND \
    'exec env ZELLIJ_SOCKET_DIR=%q %q attach %q options --on-force-close detach' \
    "$ZELLIJ_SOCKET_ROOT" "$ZELLIJ_BIN" "$BACKEND_SESSION"
}

run_config_rejection() {
  local name="$1" fixture="$2" expected="$3" root="$TEST_ROOT/config-$1"
  local output="$ARTIFACTS/config-$1.txt" status
  install -d -m 0700 "$root/.blackpepper"
  cp "$FIXTURES/$fixture" "$root/.blackpepper/config.toml"
  set +e
  (cd "$root" && timeout 10 "$BP_DEV") > "$output" 2>&1
  status=$?
  set -e
  if [ "$status" -eq 0 ] || [ "$status" -eq 124 ]; then
    fail_e2e "$name config was not rejected promptly (status $status)"
  fi
  grep -Fq -- "$expected" "$output" || {
    LAST_CAPTURE="$output"
    fail_e2e "$name config rejection was not actionable"
  }
}
