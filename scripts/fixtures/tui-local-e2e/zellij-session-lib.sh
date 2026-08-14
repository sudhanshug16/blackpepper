#!/usr/bin/env bash

# Zellij's short session list includes cached exited sessions without marking
# them as exited. The isolated socket directory is the authoritative evidence
# that a test server can still accept clients.
active_zellij_session_sockets() {
  local contract_root="$ZELLIJ_SOCKET_ROOT/contract_version_1"
  local socket
  [ -d "$contract_root" ] || return 0
  for socket in "$contract_root"/*; do
    [ -S "$socket" ] && printf '%s\n' "${socket##*/}"
  done | LC_ALL=C sort
}

wait_for_no_zellij_session_sockets() {
  local label="$1" timeout_seconds="${2:-15}"
  local attempts=$((timeout_seconds * 10))
  local attempt=0 active=''
  while [ "$attempt" -lt "$attempts" ]; do
    active="$(active_zellij_session_sockets)"
    [ -z "$active" ] && return 0
    sleep 0.1
    attempt=$((attempt + 1))
  done
  fail_e2e "$label left active Zellij session socket(s): ${active//$'\n'/, }"
}
