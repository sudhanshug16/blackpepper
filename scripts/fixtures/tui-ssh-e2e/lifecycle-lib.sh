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

kill_registered_zellij_sessions() {
  local state_root="$1" binary="$2" registry session
  registry="$state_root/blackpepper/host-registry.sqlite3"
  [ -x "$binary" ] && [ -f "$registry" ] || return 0
  while IFS= read -r session; do
    printf '%s\n' "$session" | grep -Eq \
      '^bp-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}(-[0-9a-f]{12})?$' || continue
    ZELLIJ_SOCKET_DIR="/tmp/zellij-$(id -u)" \
      "$binary" kill-session "$session" >/dev/null 2>&1 || true
  done < <(python3 - "$registry" 2>/dev/null <<'PY'
import sqlite3
import sys

with sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True) as connection:
    rows = connection.execute(
        "SELECT DISTINCT backend_session_id FROM sessions "
        "WHERE backend_json = '{\"kind\":\"zellij\"}'"
    )
    for (session,) in rows:
        print(session)
PY
  )
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

  if [ -n "${ZELLIJ_CACHE_RELATIVE:-}" ]; then
    kill_registered_zellij_sessions \
      "$TEST_ROOT/client-a/state" "$TEST_ROOT/client-a/data/$ZELLIJ_CACHE_RELATIVE/zellij"
    kill_registered_zellij_sessions \
      "$TEST_ROOT/client-b/state" "$TEST_ROOT/client-b/data/$ZELLIJ_CACHE_RELATIVE/zellij"
    kill_registered_zellij_sessions \
      "$TEST_ROOT/remote/state" "$TEST_ROOT/remote/data/$ZELLIJ_CACHE_RELATIVE/zellij"
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

load_zellij_seed() {
  local scripts_root="$1" output seed_data_home digest_file cached_digest
  local -a metadata
  output="$(
    python3 "$scripts_root/fixtures/zellij_runtime.py" \
      asset x86_64-unknown-linux-musl
  )" || fail 'could not resolve the current Zellij acceptance asset'
  mapfile -t metadata <<< "$output"
  [ "${#metadata[@]}" -eq 5 ] || fail 'current Zellij acceptance metadata is incomplete'

  ZELLIJ_VERSION="${metadata[0]}"
  ZELLIJ_CACHE_RELATIVE="${metadata[1]}"
  ZELLIJ_ASSET_NAME="${metadata[2]}"
  ZELLIJ_MANIFEST_BINARY_SHA256="${metadata[3]}"
  ZELLIJ_BINARY_NAME="${metadata[4]}"
  if [ -n "${BLACKPEPPER_E2E_ZELLIJ_SEED:-}" ]; then
    ZELLIJ_SOURCE_DIR="$BLACKPEPPER_E2E_ZELLIJ_SEED"
  else
    seed_data_home="${BLACKPEPPER_E2E_ZELLIJ_SEED_DATA_HOME:-$ORIGINAL_HOME/.local/share}"
    ZELLIJ_SOURCE_DIR="$seed_data_home/$ZELLIJ_CACHE_RELATIVE"
  fi
  ZELLIJ_SOURCE_BINARY="$ZELLIJ_SOURCE_DIR/$ZELLIJ_BINARY_NAME"
  ZELLIJ_SOURCE_ARCHIVE="$ZELLIJ_SOURCE_DIR/$ZELLIJ_ASSET_NAME"
  digest_file="$ZELLIJ_SOURCE_DIR/.zellij.sha256"

  [ -x "$ZELLIJ_SOURCE_BINARY" ] ||
    fail "verified Zellij $ZELLIJ_VERSION cache is missing: $ZELLIJ_SOURCE_BINARY"
  [ -f "$ZELLIJ_SOURCE_ARCHIVE" ] ||
    fail "verified Zellij archive is missing: $ZELLIJ_SOURCE_ARCHIVE"
  [ -f "$digest_file" ] || fail "verified Zellij binary digest is missing: $digest_file"
  cached_digest="$(tr -d '\r\n' < "$digest_file")"
  printf '%s\n' "$cached_digest" | grep -Eq '^[0-9a-f]{64}$' ||
    fail "verified Zellij binary digest is invalid: $digest_file"

  [ "$("$ZELLIJ_SOURCE_BINARY" --version)" = "zellij $ZELLIJ_VERSION" ] ||
    fail "Zellij seed is not version $ZELLIJ_VERSION: $ZELLIJ_SOURCE_BINARY"
  ZELLIJ_BINARY_SHA256="$(
    python3 "$scripts_root/fixtures/zellij_runtime.py" \
      archive-binary-sha256 x86_64-unknown-linux-musl "$ZELLIJ_SOURCE_ARCHIVE"
  )" || fail "could not verify the Zellij seed archive for $ZELLIJ_VERSION"
  [ "$cached_digest" = "$ZELLIJ_BINARY_SHA256" ] ||
    fail "Zellij cached digest does not match its pinned archive for $ZELLIJ_VERSION"
  [ "$(sha256sum "$ZELLIJ_SOURCE_BINARY" | awk '{print $1}')" = "$ZELLIJ_BINARY_SHA256" ] ||
    fail "Zellij seed binary checksum is invalid for $ZELLIJ_VERSION: $ZELLIJ_SOURCE_BINARY"
  if [ "$ZELLIJ_MANIFEST_BINARY_SHA256" != - ] &&
    [ "$ZELLIJ_BINARY_SHA256" != "$ZELLIJ_MANIFEST_BINARY_SHA256" ]; then
    fail "Zellij seed binary checksum is not manifest-pinned for $ZELLIJ_VERSION"
  fi
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
