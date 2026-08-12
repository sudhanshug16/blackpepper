#!/usr/bin/env bash

# Process, dependency, and timeout helpers for test-tui-ssh-e2e.sh. The caller
# owns all paths and PIDs so this file remains reusable without hidden state.

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  if [ -n "$TMUX_SESSION" ] && tmux -S "$TMUX_SOCKET" has-session -t "$TMUX_SESSION" 2>/dev/null; then
    printf '%s\n' '--- Blackpepper screen ---' >&2
    tmux -S "$TMUX_SOCKET" capture-pane -p -J -t "$TMUX_SESSION:0.0" >&2 || true
  fi
  if [ -s "$SSH_LOG" ]; then
    printf '%s\n' '--- SSH argv (last 20) ---' >&2
    tail -n 20 "$SSH_LOG" | cut -c1-600 >&2 || true
  fi
  if [ -s "$SSHD_LOG" ]; then
    printf '%s\n' '--- sshd log (last 30) ---' >&2
    tail -n 30 "$SSHD_LOG" >&2 || true
  fi
  exit 1
}

kill_zellij_tree() {
  local runtime_root="$1" data_root="$2" binary="$3"
  [ -x "$binary" ] || return 0
  env \
    XDG_RUNTIME_DIR="$runtime_root" \
    XDG_DATA_HOME="$data_root" \
    XDG_CONFIG_HOME="$TEST_ROOT/empty-config" \
    "$binary" kill-all-sessions --yes >/dev/null 2>&1 || true
}

cleanup() {
  local status=$?
  set +e

  if [ -n "$TMUX_SESSION" ]; then
    tmux -S "$TMUX_SOCKET" kill-session -t "$TMUX_SESSION" >/dev/null 2>&1 || true
  fi
  tmux -S "$TMUX_SOCKET" kill-server >/dev/null 2>&1 || true

  # A forced test failure can stop the client before Rust drops its TempDir.
  # Remove only socket paths allocated by this test's recorded ssh argv.
  if [ -s "$SSH_LOG" ]; then
    while IFS= read -r socket; do
      case "$socket" in
        /tmp/bp-ssh-??????/c)
          rm -f -- "$socket"
          rmdir -- "${socket%/c}" >/dev/null 2>&1 || true
          ;;
      esac
    done < <(grep -Eo '/tmp/bp-ssh-[^ /]+/c' "$SSH_LOG" | sort -u)
  fi

  if [ -n "$LISTENER_PID" ]; then
    kill "$LISTENER_PID" >/dev/null 2>&1 || true
    wait "$LISTENER_PID" >/dev/null 2>&1 || true
  fi

  if [ -n "${ZELLIJ_SOURCE_BINARY:-}" ]; then
    kill_zellij_tree "$TEST_ROOT/client-a/runtime" "$TEST_ROOT/client-a/data" "$ZELLIJ_SOURCE_BINARY"
    kill_zellij_tree "$TEST_ROOT/client-b/runtime" "$TEST_ROOT/client-b/data" "$ZELLIJ_SOURCE_BINARY"
    kill_zellij_tree "$TEST_ROOT/remote/runtime" "$TEST_ROOT/remote/data" "$ZELLIJ_SOURCE_BINARY"
  fi

  if [ -n "$SSHD_PID" ]; then
    kill "$SSHD_PID" >/dev/null 2>&1 || true
    wait "$SSHD_PID" >/dev/null 2>&1 || true
  fi

  if [ "${BLACKPEPPER_E2E_KEEP:-0}" = 1 ]; then
    printf 'Blackpepper SSH E2E artifacts retained at %s\n' "$TEST_ROOT" >&2
  else
    find "$TEST_ROOT" -depth -delete >/dev/null 2>&1 || true
  fi
  if [ "$FAILED" -eq 0 ]; then
    return 0
  fi
  return "$status"
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 is required"
}

resolve_bp_binary() {
  local candidate bundle
  if [ -n "${BLACKPEPPER_E2E_BP:-}" ]; then
    candidate="$BLACKPEPPER_E2E_BP"
  else
    candidate="$(command -v bp-dev 2>/dev/null || true)"
    [ -n "$candidate" ] || fail 'bp-dev is not installed; run scripts/setup.sh'
    bundle="$(dirname "$candidate")/.blackpepper-dev/current/bp-dev"
    if [ -x "$bundle" ]; then
      candidate="$bundle"
    fi
  fi
  [ -x "$candidate" ] || fail "Blackpepper binary is not executable: $candidate"
  readlink -f -- "$candidate"
}

choose_port() {
  python3 - <<'PY'
import socket
with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
}

wait_until() {
  local description="$1" timeout="$2"
  shift 2
  local deadline=$((SECONDS + timeout))
  while [ "$SECONDS" -lt "$deadline" ]; do
    if "$@"; then
      return 0
    fi
    sleep 0.1
  done
  fail "timed out waiting for $description"
}

tcp_port_open() {
  nc -z "$1" "$2" >/dev/null 2>&1
}
