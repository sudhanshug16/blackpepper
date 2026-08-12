#!/usr/bin/env bash

# Stable, Blackpepper-owned v2 mode anchors. Terminal output must never be
# treated as a mode marker on its own.
BLACKPEPPER_MANAGE_MARKER=' MANAGE '
BLACKPEPPER_TERMINAL_ANCHOR='bp  '

capture_is_manage_mode() {
  grep -Fq -- "$BLACKPEPPER_MANAGE_MARKER" "$LAST_CAPTURE"
}

capture_is_terminal_mode() {
  tail -n 1 "$LAST_CAPTURE" | grep -Fq -- "$BLACKPEPPER_TERMINAL_ANCHOR" &&
    ! grep -Fq -- "$BLACKPEPPER_MANAGE_MARKER" "$LAST_CAPTURE" &&
    ! grep -Fq -- ' AUTHENTICATE ' "$LAST_CAPTURE"
}

wait_for_terminal_mode() {
  local label="$1" timeout_seconds="${2:-20}"
  local attempts=$((timeout_seconds * 10))
  local attempt=0
  while [ "$attempt" -lt "$attempts" ]; do
    capture_screen "$label"
    if capture_is_terminal_mode; then
      return 0
    fi
    if ! session_is_live; then
      fail_e2e 'TUI exited while waiting for the Blackpepper terminal status row'
    fi
    sleep 0.1
    attempt=$((attempt + 1))
  done
  fail_e2e "timed out after ${timeout_seconds}s waiting for the Blackpepper terminal status row"
}
