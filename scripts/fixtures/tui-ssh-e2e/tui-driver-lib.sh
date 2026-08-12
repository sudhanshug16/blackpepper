#!/usr/bin/env bash

# Tmux/PTY driver and fixture assertions for test-tui-ssh-e2e.sh. The caller
# provides the isolated XDG roots and exact Blackpepper/Zellij binaries.

seed_zellij_cache() {
  local destination="$1/$ZELLIJ_CACHE_RELATIVE"
  install -d -m 0700 "$destination"
  cp "$ZELLIJ_SOURCE_BINARY" "$destination/zellij"
  cp "$ZELLIJ_SOURCE_ARCHIVE" "$destination/zellij-x86_64-unknown-linux-musl.tar.gz"
  printf '%s\n' "$ZELLIJ_BINARY_SHA256" > "$destination/.zellij.sha256"
  chmod 0700 "$destination/zellij"
  chmod 0600 "$destination/zellij-x86_64-unknown-linux-musl.tar.gz" "$destination/.zellij.sha256"
}

prepare_client() {
  local label="$1" client="$TEST_ROOT/$1"
  install -d -m 0700 \
    "$client/home/.ssh" \
    "$client/config" \
    "$client/data" \
    "$client/state" \
    "$client/runtime" \
    "$client/tmp" \
    "$client/cwd"
  cp "$SSH_CONFIG" "$client/home/.ssh/config"
  chmod 0600 "$client/home/.ssh/config"
  seed_zellij_cache "$client/data"
  printf '%s\n' "$label" > "$client/label"
}

prepare_tui_fixtures() {
  prepare_client client-a
  prepare_client client-b
  # Default to a checksum-pinned remote seed so the primary test measures SSH
  # control/PTY behavior. Set BLACKPEPPER_E2E_PRESEED_REMOTE=0 to exercise the
  # real managed-sidecar upload and remote verification path as well.
  if [ "${BLACKPEPPER_E2E_PRESEED_REMOTE:-1}" != 0 ]; then
    seed_zellij_cache "$TEST_ROOT/remote/data"
  fi

  cat > "$TEST_ROOT/run-client" <<'EOF'
#!/usr/bin/env bash
set +e
"$BLACKPEPPER_E2E_BP_BINARY"
status=$?
printf '\nBLACKPEPPER_E2E_EXIT:%s\n' "$status"
sleep 300
exit "$status"
EOF
  chmod 0700 "$TEST_ROOT/run-client"
}

sidecar_mode() {
  if [ "${BLACKPEPPER_E2E_PRESEED_REMOTE:-1}" = 0 ]; then
    printf '%s' 'managed Zellij upload and remote SHA verification'
  else
    printf '%s' 'checksum-pinned remote Zellij seed'
  fi
}

assert_remote_sidecars() {
  local sidecars="$TEST_ROOT/remote/data/blackpepper/sidecars"
  local zellij="$sidecars/zellij/0.44.3/x86_64-unknown-linux-musl/zellij"
  local remote_bp_host

  [ -x "$zellij" ] || fail 'remote Zellij sidecar is missing or not executable'
  [ "$(stat -c '%a' "$zellij")" = 700 ] || fail 'remote Zellij sidecar permissions are not 0700'
  [ "$(sha256sum "$zellij" | awk '{print $1}')" = "$ZELLIJ_BINARY_SHA256" ] ||
    fail 'remote Zellij sidecar checksum does not match the pinned release'

  [ -d "$sidecars/bp-host" ] || fail 'remote bp-host sidecar directory was not installed'
  remote_bp_host="$(find "$sidecars/bp-host" -type f -name bp-host -print -quit)"
  [ -n "$remote_bp_host" ] || fail 'remote bp-host sidecar was not installed'
  [ "$(stat -c '%a' "$remote_bp_host")" = 700 ] || fail 'remote bp-host permissions are not 0700'
  [ "$(sha256sum "$remote_bp_host" | awk '{print $1}')" = "$(sha256sum "$BP_HOST_BINARY" | awk '{print $1}')" ] ||
    fail 'remote bp-host sidecar differs from the tested client bundle'
  [ -z "$(find "$sidecars" -type f -name '*.upload' -print -quit)" ] ||
    fail 'a partial remote sidecar upload remained after provisioning'
}

capture_screen() {
  tmux -S "$TMUX_SOCKET" capture-pane -p -J -t "$TMUX_SESSION:0.0"
}

screen_has() {
  capture_screen 2>/dev/null | grep -Fq -- "$1"
}

screen_matches() {
  capture_screen 2>/dev/null | grep -Eq -- "$1"
}

screen_lacks() {
  ! screen_has "$1"
}

start_client() {
  local label="$1" client="$TEST_ROOT/$1" command
  TMUX_SESSION="$label"
  printf -v command \
    'exec env -i HOME=%q USER=%q LOGNAME=%q SHELL=/bin/bash TERM=xterm-256color LANG=C.UTF-8 PATH=%q TMPDIR=%q HOSTNAME=%q XDG_CONFIG_HOME=%q XDG_DATA_HOME=%q XDG_STATE_HOME=%q XDG_RUNTIME_DIR=%q BLACKPEPPER_E2E_SSH_CONFIG=%q BLACKPEPPER_E2E_SSH_LOG=%q BLACKPEPPER_E2E_BP_BINARY=%q %q' \
    "$client/home" "$(id -un)" "$(id -un)" \
    "$TEST_ROOT/bin:/usr/bin:/bin" "$client/tmp" "$label" \
    "$client/config" "$client/data" "$client/state" "$client/runtime" \
    "$SSH_CONFIG" "$SSH_LOG" "$BP_BINARY" "$TEST_ROOT/run-client"
  tmux -S "$TMUX_SOCKET" new-session -d -s "$TMUX_SESSION" -x 150 -y 42 -c "$client/cwd" "$command"
  wait_until "$label startup screen" 30 screen_has 'Hosts / Workspaces'
  wait_until "$label manage mode" 10 screen_has 'MANAGE'
}

stop_client() {
  send_command ':quit'
  wait_until "$TMUX_SESSION clean exit" 20 screen_has 'BLACKPEPPER_E2E_EXIT:0'
  tmux -S "$TMUX_SOCKET" kill-session -t "$TMUX_SESSION" >/dev/null 2>&1 || true
  TMUX_SESSION=''
}

send_literal() {
  tmux -S "$TMUX_SOCKET" send-keys -t "$TMUX_SESSION:0.0" -l -- "$1"
}

send_enter() {
  tmux -S "$TMUX_SOCKET" send-keys -t "$TMUX_SESSION:0.0" Enter
}

ensure_manage() {
  if screen_has 'MANAGE'; then
    return 0
  fi
  if screen_has 'WORK'; then
    tmux -S "$TMUX_SOCKET" send-keys -t "$TMUX_SESSION:0.0" -H 1d
    wait_until 'manage mode' 10 screen_has 'MANAGE'
    return 0
  fi
  fail 'Blackpepper is neither in work nor manage mode'
}

ensure_work() {
  if screen_has 'WORK'; then
    return 0
  fi
  ensure_manage
  tmux -S "$TMUX_SOCKET" send-keys -t "$TMUX_SESSION:0.0" Escape
  wait_until 'work mode' 10 screen_has 'WORK'
}

dismiss_zellij_tip() {
  local attempt=0

  # The tip is painted shortly after the PTY first becomes renderable. An
  # immediate one-shot check races that paint and sends the next shell command
  # into the modal. Let Zellij settle, then disable tips only in this test's
  # isolated XDG roots and prove the overlay is gone before driving the shell.
  sleep 1
  while [ "$attempt" -lt 3 ]; do
    attempt=$((attempt + 1))
    if screen_has 'About Zellij'; then
      tmux -S "$TMUX_SOCKET" send-keys -t "$TMUX_SESSION:0.0" C-c
      wait_until 'Zellij first-use tip dismissal' 10 screen_lacks 'About Zellij'
      sleep 0.2
      continue
    fi
    return 0
  done
  fail 'Zellij first-use tip remained visible after dismissal'
}

send_command() {
  ensure_manage
  send_literal "$1"
  send_enter
}

save_screen() {
  capture_screen > "$TEST_ROOT/$1.screen"
}

assert_registry() {
  local database="$1" workspace="$2" expected_state="$3"
  python3 - "$database" "$workspace" "$expected_state" <<'PY'
import sqlite3
import sys

database, workspace, expected_state = sys.argv[1:]
connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
workspace_rows = connection.execute(
    "SELECT id, host_id FROM workspaces WHERE root_path = ?", (workspace,)
).fetchall()
if len(workspace_rows) != 1:
    raise SystemExit(f"expected one remote workspace row, got {workspace_rows!r}")
sessions = connection.execute(
    "SELECT backend_version, state_json FROM sessions WHERE workspace_id = ?",
    (workspace_rows[0][0],),
).fetchall()
if not sessions:
    raise SystemExit("remote workspace has no persistent session row")
if not any(version == "0.44.3" and expected_state in state for version, state in sessions):
    raise SystemExit(f"remote session does not include version 0.44.3 / {expected_state}: {sessions!r}")
PY
}
