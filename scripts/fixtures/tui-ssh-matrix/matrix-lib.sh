#!/usr/bin/env bash

# Isolated OpenSSH server lifecycle and security assertions for the live
# ProxyJump / authentication matrix. The caller owns all paths and globals.

matrix_fail() {
  printf 'FAIL: %s\n' "$*" >&2
  if [ -n "${TMUX_SESSION:-}" ] &&
    tmux -S "$TMUX_SOCKET" has-session -t "$TMUX_SESSION" 2>/dev/null; then
    printf '%s\n' '--- Blackpepper screen ---' >&2
    tmux -S "$TMUX_SOCKET" capture-pane -p -J -t "$TMUX_SESSION:0.0" >&2 || true
  fi
  for log in "${SSH_LOG:-}" "${JUMP_LOG:-}" "${TARGET_LOG:-}"; do
    [ -s "$log" ] || continue
    printf '%s\n' "--- $(basename "$log") (last 30) ---" >&2
    tail -n 30 "$log" | cut -c1-700 >&2 || true
  done
  exit 1
}

# Override the base fixture's failure hook so diagnostics include both sshds.
fail() {
  matrix_fail "$@"
}

stop_process() {
  local pid="$1"
  [ -n "$pid" ] || return 0
  kill "$pid" >/dev/null 2>&1 || true
  wait "$pid" >/dev/null 2>&1 || true
}

matrix_cleanup() {
  local status=$?
  set +e

  if [ -n "${TMUX_SESSION:-}" ]; then
    tmux -S "$TMUX_SOCKET" kill-session -t "$TMUX_SESSION" >/dev/null 2>&1 || true
  fi
  tmux -S "$TMUX_SOCKET" kill-server >/dev/null 2>&1 || true

  if [ -s "${SSH_LOG:-}" ]; then
    while IFS= read -r socket; do
      case "$socket" in
        /tmp/bp-ssh-??????/c)
          rm -f -- "$socket"
          rmdir -- "${socket%/c}" >/dev/null 2>&1 || true
          ;;
      esac
    done < <(grep -Eo '/tmp/bp-ssh-[^ /]+/c' "$SSH_LOG" | sort -u)
  fi

  if [ -n "${ZELLIJ_SOURCE_BINARY:-}" ]; then
    kill_zellij_tree \
      "$TEST_ROOT/client-a/runtime" \
      "$TEST_ROOT/client-a/data" \
      "$ZELLIJ_SOURCE_BINARY"
    kill_zellij_tree \
      "$TEST_ROOT/remote/runtime" \
      "$TEST_ROOT/remote/data" \
      "$ZELLIJ_SOURCE_BINARY"
  fi

  stop_process "${TARGET_PID:-}"
  stop_process "${JUMP_PID:-}"

  if [ "${BLACKPEPPER_E2E_KEEP:-0}" = 1 ]; then
    printf 'Blackpepper SSH matrix artifacts retained at %s\n' "$TEST_ROOT" >&2
  else
    find "$TEST_ROOT" -depth -delete >/dev/null 2>&1 || true
  fi
  if [ "${FAILED:-1}" -eq 0 ]; then
    return 0
  fi
  return "$status"
}

write_sshd_config() {
  local destination="$1" port="$2" host_key="$3" log_level="${4:-VERBOSE}"
  apply_sshd_config "$destination" "$port" "$host_key" "$log_level"
}

# Kept separate so ShellCheck can verify all expansions in the heredoc owner.
apply_sshd_config() {
  local destination="$1" port="$2" host_key="$3" log_level="$4"
  local user
  user="$(id -un)"
  {
    printf 'Port %s\n' "$port"
    printf '%s\n' 'ListenAddress 127.0.0.1'
    printf 'HostKey %s\n' "$host_key"
    printf 'PidFile %s.pid\n' "$destination"
    printf 'AuthorizedKeysFile %s\n' "$TEST_ROOT/sshd/authorized_keys"
    printf '%s\n' 'PasswordAuthentication no'
    printf '%s\n' 'KbdInteractiveAuthentication no'
    printf '%s\n' 'PubkeyAuthentication yes'
    printf '%s\n' 'PermitRootLogin no'
    printf '%s\n' 'UsePAM no'
    printf 'AllowUsers %s\n' "$user"
    printf '%s\n' 'StrictModes no'
    printf '%s\n' 'UseDNS no'
    printf '%s\n' 'PrintMotd no'
    printf 'LogLevel %s\n' "$log_level"
    printf '%s\n' 'AllowTcpForwarding yes'
    printf '%s\n' 'PermitTunnel no'
    printf '%s\n' 'AcceptEnv XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME XDG_RUNTIME_DIR'
  } > "$destination"
  chmod 0600 "$destination"
}

start_sshd() {
  local config="$1" log="$2" pid_name="$3" port="$4" pid
  "$SSHD" -t -f "$config"
  "$SSHD" -D -e -f "$config" >> "$log" 2>&1 &
  pid=$!
  printf -v "$pid_name" '%s' "$pid"
  wait_until "temporary sshd on port $port" 10 tcp_port_open 127.0.0.1 "$port"
}

accepted_connections() {
  local log="$1"
  grep -c '^Accepted publickey ' "$log" 2>/dev/null || true
}

network_connections() {
  local log="$1"
  grep -c '^Connection from ' "$log" 2>/dev/null || true
}

assert_connection_counts() {
  local expected_jump="$1" expected_target="$2" context="$3"
  local actual_jump actual_target
  actual_jump="$(accepted_connections "$JUMP_LOG")"
  actual_target="$(accepted_connections "$TARGET_LOG")"
  [ "$actual_jump" -eq "$expected_jump" ] ||
    fail "$context: expected $expected_jump authenticated jump connection(s), got $actual_jump"
  [ "$actual_target" -eq "$expected_target" ] ||
    fail "$context: expected $expected_target authenticated target connection(s), got $actual_target"
}

screen_count_at_least() {
  local needle="$1" expected="$2" count
  count="$(capture_screen 2>/dev/null | grep -Fo -- "$needle" | wc -l)"
  [ "$count" -ge "$expected" ]
}

assert_fail_closed_without_mux() {
  local accepted_jump accepted_target network_jump network_target stderr_file
  accepted_jump="$(accepted_connections "$JUMP_LOG")"
  accepted_target="$(accepted_connections "$TARGET_LOG")"
  network_jump="$(network_connections "$JUMP_LOG")"
  network_target="$(network_connections "$TARGET_LOG")"
  stderr_file="$TEST_ROOT/fail-closed.stderr"

  if env HOME="$TEST_ROOT/client-a/home" /usr/bin/ssh \
    -o ControlMaster=no \
    -o ControlPersist=no \
    -o ProxyJump=none \
    -o ProxyCommand=false \
    -o CanonicalizeHostname=no \
    -o BatchMode=yes \
    -o ConnectTimeout=3 \
    -F "$SSH_CONFIG" \
    -S "$TEST_ROOT/missing-mux" \
    -T -- bp-proxy-e2e true > /dev/null 2> "$stderr_file"; then
    fail 'a fail-closed SSH child succeeded without its owned mux socket'
  fi
  # Some OpenSSH builds return the ProxyCommand's non-zero status without a
  # diagnostic. The security property is the failed command plus unchanged
  # sshd authentication counts, not a version-specific stderr sentence.
  assert_connection_counts "$accepted_jump" "$accepted_target" \
    'fail-closed child attempted a direct network connection'
  [ "$(network_connections "$JUMP_LOG")" -eq "$network_jump" ] ||
    fail 'missing-mux child opened a new TCP connection to the jump host'
  [ "$(network_connections "$TARGET_LOG")" -eq "$network_target" ] ||
    fail 'missing-mux child opened a new TCP connection to the target host'
}
