#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)"
FIXTURES="$ROOT/scripts/fixtures/tui-local-e2e"
# shellcheck source=scripts/fixtures/tui-local-e2e/harness-lib.sh
source "$FIXTURES/harness-lib.sh"
# shellcheck source=scripts/fixtures/tui-local-e2e/zellij-session-lib.sh
source "$FIXTURES/zellij-session-lib.sh"

for requirement in tmux git python3 ssh curl ss timeout; do
  command -v "$requirement" >/dev/null 2>&1 || {
    printf 'FAIL: required command is unavailable: %s\n' "$requirement" >&2
    exit 1
  }
done
ZELLIJ_VERSION="$(python3 "$ROOT/scripts/fixtures/zellij_runtime.py" version)"

if [ "$(uname -s)" != Linux ]; then
  printf '%s\n' 'FAIL: this local acceptance harness currently requires Linux port attribution.' >&2
  exit 1
fi

BP_LAUNCHER="${BLACKPEPPER_TUI_E2E_BP_DEV:-$(command -v bp-dev 2>/dev/null || true)}"
if [ -z "$BP_LAUNCHER" ] || [ ! -x "$BP_LAUNCHER" ]; then
  printf '%s\n' 'FAIL: globally installed bp-dev was not found; run scripts/setup.sh first.' >&2
  exit 1
fi

if [ -z "${BLACKPEPPER_TUI_E2E_BP_DEV:-}" ]; then
  LAUNCHER_DIR="$(CDPATH='' cd -P -- "$(dirname "$BP_LAUNCHER")" && pwd)"
  CURRENT_BUNDLE="$LAUNCHER_DIR/.blackpepper-dev/current"
  [ -d "$CURRENT_BUNDLE" ] || {
    printf 'FAIL: bp-dev launcher has no installed current bundle: %s\n' "$CURRENT_BUNDLE" >&2
    exit 1
  }
  CURRENT_BUNDLE="$(CDPATH='' cd -P -- "$CURRENT_BUNDLE" && pwd)"
  BP_DEV="$CURRENT_BUNDLE/bp-dev"
else
  BP_DEV="$BP_LAUNCHER"
fi
if [ ! -f "$BP_DEV" ] || [ ! -x "$BP_DEV" ]; then
  printf 'FAIL: installed bp-dev executable is invalid: %s\n' "$BP_DEV" >&2
  exit 1
fi
BP_VERSION="$($BP_DEV --version 2>&1)"
case "$BP_VERSION" in
  'blackpepper '*'-dev.'*) ;;
  *) printf 'FAIL: expected a development build, got: %s\n' "$BP_VERSION" >&2; exit 1 ;;
esac

# Zellij's Unix socket has a 107-byte kernel limit, including its own
# contract/session suffix. Keep the isolated development socket root short.
TEST_ROOT="$(mktemp -d /tmp/bpl.XXXXXX)"
ARTIFACTS="$TEST_ROOT/artifacts"
PRIMARY="$TEST_ROOT/primary"
PEER="$TEST_ROOT/peer"
CREATED="$TEST_ROOT/worktrees/e2e-created"
TEMP_HOME="$TEST_ROOT/home"
TMUX_SOCKET="$TEST_ROOT/tmux.sock"
SECOND_TMUX_SOCKET="$TEST_ROOT/tmux-second.sock"
TMUX_SESSION='bp-tui-e2e'
LISTENER_PID=''
ZELLIJ_BIN=''
ZELLIJ_SOCKET_ROOT="$TEST_ROOT/z"
E2E_DATA_HOME="${BLACKPEPPER_TUI_E2E_DATA_HOME:-$ROOT/target/tui-local-e2e-data}"
ZELLIJ_CACHE_ROOT="$E2E_DATA_HOME/blackpepper/sidecars/zellij/$ZELLIJ_VERSION"

case "$E2E_DATA_HOME" in
  /*) ;;
  *) printf 'FAIL: BLACKPEPPER_TUI_E2E_DATA_HOME must be absolute: %s\n' "$E2E_DATA_HOME" >&2; exit 1 ;;
esac

# Preserve a signal's nonzero status through one EXIT cleanup invocation.
trap cleanup_e2e EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

install -d -m 0700 \
  "$ARTIFACTS" "$PRIMARY/.blackpepper" "$TEMP_HOME/.ssh" \
  "$TEST_ROOT/config/worktrunk" "$TEST_ROOT/state" "$TEST_ROOT/run" \
  "$TEST_ROOT/cache" "$ZELLIJ_SOCKET_ROOT" "$E2E_DATA_HOME"
EMPTY_ZELLIJ_SOCKET_CHECK=''
if ! EMPTY_ZELLIJ_SOCKET_CHECK="$(active_zellij_session_sockets)"; then
  fail_e2e 'empty Zellij socket inspection returned a failure status'
fi
[ -z "$EMPTY_ZELLIJ_SOCKET_CHECK" ] ||
  fail_e2e "new isolated Zellij socket root was not empty: $EMPTY_ZELLIJ_SOCKET_CHECK"
cp "$FIXTURES/config.toml" "$PRIMARY/.blackpepper/config.toml"
printf '%s\n' '# Blackpepper local TUI acceptance fixture' > "$PRIMARY/README.md"
git -C "$PRIMARY" init --initial-branch=main --quiet
git -C "$PRIMARY" config user.name 'Blackpepper E2E'
git -C "$PRIMARY" config user.email 'blackpepper-e2e@example.invalid'
git -C "$PRIMARY" add README.md .blackpepper/config.toml
git -C "$PRIMARY" commit --quiet -m 'test: initialize local TUI fixture'
git -C "$PRIMARY" remote add origin 'https://example.invalid/acme/e2e.git'
git -C "$PRIMARY" worktree add --quiet -b peer "$PEER"

printf 'worktree-path = "%s/worktrees/{{ branch | sanitize }}"\n' "$TEST_ROOT" \
  > "$TEST_ROOT/config/worktrunk/config.toml"
printf '%s\n' \
  'Host e2e-host' \
  '    HostName 127.0.0.1' \
  '    User blackpepper-e2e' \
  '' \
  'Host ignored-*' \
  '    HostName 192.0.2.1' \
  > "$TEMP_HOME/.ssh/config"
chmod 0600 "$TEMP_HOME/.ssh/config"

PORT_FILE="$TEST_ROOT/listener.port"
AUTO_SERVICE_LOG_ROOT="$TEST_ROOT/auto-service-starts"
AUTO_SERVICE_LOG=""
(
  cd "$PRIMARY"
  exec python3 "$FIXTURES/listener.py" "$PORT_FILE"
) > "$ARTIFACTS/listener.log" 2>&1 &
LISTENER_PID=$!
for _attempt in $(seq 1 100); do
  [ -s "$PORT_FILE" ] && break
  kill -0 "$LISTENER_PID" 2>/dev/null || fail_e2e 'fixture listener exited during startup'
  sleep 0.1
done
[ -s "$PORT_FILE" ] || fail_e2e 'fixture listener did not publish its port'
REMOTE_PORT="$(tr -d '\r\n' < "$PORT_FILE")"
case "$REMOTE_PORT" in
  ''|*[!0-9]*) fail_e2e "fixture listener returned invalid port: $REMOTE_PORT" ;;
esac

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
export BLACKPEPPER_E2E_AUTO_LOG_ROOT="$AUTO_SERVICE_LOG_ROOT"
install -d -m 0700 "$BLACKPEPPER_E2E_AUTO_LOG_ROOT"

tmux -S "$TMUX_SOCKET" new-session -d -s "$TMUX_SESSION" -x 180 -y 52 \
  -c "$PRIMARY" "$BP_DEV"
wait_for_screen 'bp  blackpepper' startup 60
assert_screen_has 'HOSTS' startup-hosts
assert_screen_has 'SESSION' startup-session
assert_screen_has 'PORTS' startup-ports
wait_for_screen 'primary' startup-primary 60
assert_screen_has 'acme/e2e' startup-grouping
assert_screen_lacks 'os error 2' startup-no-enoent

for _attempt in $(seq 1 200); do
  ZELLIJ_BIN="$(find "$ZELLIJ_CACHE_ROOT" \
    -type f -name zellij -perm -u+x -print -quit 2>/dev/null || true)"
  [ -n "$ZELLIJ_BIN" ] && break
  sleep 0.1
done
[ -n "$ZELLIJ_BIN" ] || fail_e2e "managed Zellij $ZELLIJ_VERSION was not installed"
[ "$($ZELLIJ_BIN --version)" = "zellij $ZELLIJ_VERSION" ] ||
  fail_e2e 'managed Zellij version is not exact'
prepare_backend_session

set +e
(
  cd "$PRIMARY"
  timeout 10 "$BP_DEV"
) > "$ARTIFACTS/singleton.txt" 2>&1
SECOND_STATUS=$?
set -e
if [ "$SECOND_STATUS" -eq 0 ] || [ "$SECOND_STATUS" -eq 124 ]; then
  fail_e2e "second bp-dev did not fail promptly (status $SECOND_STATUS)"
fi
grep -Fq 'Blackpepper is already running as PID' "$ARTIFACTS/singleton.txt" ||
  fail_e2e 'singleton rejection did not report the owner PID'

send_enter
wait_for_terminal_mode attached-terminal 15
wait_for_zellij_client_count "$BACKEND_SESSION" 1 15
for _attempt in $(seq 1 100); do
  AUTO_SERVICE_LOG="$(find "$AUTO_SERVICE_LOG_ROOT" -type f -print -quit 2>/dev/null || true)"
  [ -n "$AUTO_SERVICE_LOG" ] && break
  sleep 0.1
done
[ -n "$AUTO_SERVICE_LOG" ] || fail_e2e 'auto-start service did not publish its launch log'
wait_for_file_lines "$AUTO_SERVICE_LOG" 'workspace-env-ok' 1 10
dismiss_zellij_popups
run_shell_command "printf \"\\nBP_E2E_SHELL:%s\\n\" \"\$BLACKPEPPER_E2E\""
wait_for_screen 'BP_E2E_SHELL:workspace-env-ok' shell-passthrough 10

start_second_zellij_client
wait_for_zellij_client_count "$BACKEND_SESSION" 2 15
wait_for_screen '2 clients' attached-client-count 10
tmux -S "$SECOND_TMUX_SOCKET" kill-server
wait_for_zellij_client_count "$BACKEND_SESSION" 1 15

run_tui_command ':host import'
wait_for_screen 'SSH import preview' host-import 10
assert_screen_has 'e2e-host → e2e-host' host-import-alias
run_tui_command ':definitely-not-a-command'
wait_for_screen 'Unknown command: :definitely-not-a-command' invalid-command 5
send_escape
wait_for_screen_absent ':definitely-not-a-command' invalid-command-closed 5
run_tui_command ':help'
wait_for_screen ':host add <name> <ssh-alias>' help 5
send_escape
wait_for_screen_absent ':host add <name> <ssh-alias>' help-closed 5
run_tui_command ':refresh'
wait_for_screen 'Refreshed 0 connected remote host(s)' refresh 15
run_tui_command ':status explain'
wait_for_screen 'No agent run is registered for this workspace.' status-empty 5

run_tui_command ':ports'
wait_for_screen 'listening port(s) for workspace attribution' ports 15
assert_screen_has "127.0.0.1:$REMOTE_PORT" ports-listener
run_tui_command ":forward $REMOTE_PORT"
wait_for_screen "Local service is already available at http://127.0.0.1:$REMOTE_PORT" forward 10
curl -fsS "http://127.0.0.1:$REMOTE_PORT/" | grep -Fq 'blackpepper-tui-e2e' ||
  fail_e2e 'forwarded local service was not reachable'
run_tui_command ":forward cancel $REMOTE_PORT"
wait_for_screen "Removed the local URL shortcut for 127.0.0.1:$REMOTE_PORT" forward-cancel 10

start_second_zellij_client
wait_for_zellij_client_count "$BACKEND_SESSION" 2 15
wait_for_screen '2 clients' service-multi-client-count 10
run_tui_command ':service start fixture'
wait_for_screen 'refusing BackgroundMutation: Zellij session has 2 controlling client(s)' service-multi-client-refused 10
[ ! -e "$PRIMARY/.bp-e2e-service-started" ] ||
  fail_e2e 'multi-client service refusal still launched the service'
tmux -S "$SECOND_TMUX_SOCKET" kill-server
wait_for_zellij_client_count "$BACKEND_SESSION" 1 15

run_tui_command ':service start fixture'
wait_for_screen 'Service fixture is running in background Zellij tab' service 15
wait_for_file_text "$PRIMARY/.bp-e2e-service-started" 'workspace-env-ok' 10
capture_screen service-no-focus-theft
if ! grep -Fq 'BP_E2E_SHELL:workspace-env-ok' "$LAST_CAPTURE"; then
  if [ "${BLACKPEPPER_TUI_E2E_ALLOW_FOCUS_THEFT:-0}" != 1 ]; then
    fail_e2e 'background service creation stole the attached client focus'
  fi
  # Development-only continuation for discovering failures after this known
  # blocker. Native Zellij Tab mode + 1 returns the sole client to tab one.
  ensure_work_mode
  send_hex 14
  send_literal '1'
  wait_for_screen 'BP_E2E_SHELL:workspace-env-ok' service-focus-restored 10
fi

run_tui_command ':worktree list'
wait_for_screen 'main:' worktree-list 60
assert_screen_has 'peer:' worktree-list-peer
WT_BIN="$(find "$E2E_DATA_HOME/blackpepper/sidecars/worktrunk/0.72.0" \
  -type f -name wt -perm -u+x -print -quit 2>/dev/null || true)"
if [ -z "$WT_BIN" ] || [ "$($WT_BIN --version)" != 'wt v0.72.0' ]; then
  fail_e2e 'managed Worktrunk 0.72.0 was not installed exactly'
fi

run_tui_command ":workspace add $PEER"
wait_for_terminal_mode peer-attached 20
dismiss_zellij_popups
ensure_manage_mode
assert_screen_has 'primary' grouped-primary
assert_screen_has 'peer' grouped-peer
assert_screen_has 'acme/e2e' grouped-repository
ensure_work_mode
run_shell_command "printf \"\\nBP_E2E_PEER:%s\\n\" \"\$PWD\""
wait_for_screen "BP_E2E_PEER:$PEER" peer-shell 10
for _attempt in $(seq 1 100); do
  AUTO_SERVICE_LOG_COUNT="$(find "$AUTO_SERVICE_LOG_ROOT" -type f | wc -l | tr -d '[:space:]')"
  [ "$AUTO_SERVICE_LOG_COUNT" -eq 2 ] && break
  sleep 0.1
done
[ "$AUTO_SERVICE_LOG_COUNT" -eq 2 ] || fail_e2e 'peer auto-start service did not publish a distinct launch log'
PEER_AUTO_SERVICE_LOG="$(find "$AUTO_SERVICE_LOG_ROOT" -type f ! -path "$AUTO_SERVICE_LOG" -print -quit)"
wait_for_file_lines "$PEER_AUTO_SERVICE_LOG" 'workspace-env-ok' 1 10

run_tui_command ':workspace ungroup'
wait_for_screen 'outside automatic repository grouping' ungroup 10
assert_screen_has 'folder' ungroup-folder
run_tui_command ':refresh'
wait_for_screen 'Refreshed 0 connected remote host(s)' ungroup-refresh 15
assert_screen_has 'folder' ungroup-persistent

send_hex 0e
wait_for_terminal_mode chord-switch-terminal 15
dismiss_zellij_popups
run_shell_command "printf \"\\nBP_E2E_SWITCH:%s\\n\" \"\$PWD\""
wait_for_screen "BP_E2E_SWITCH:$PRIMARY" chord-switch-primary 10
send_hex 1c
wait_for_screen ' MANAGE ' workspace-overlay 10
tmux_e2e send-keys -t "$TMUX_SESSION:0.0" Down
sleep 0.2
send_enter
wait_for_terminal_mode workspace-overlay-attach 15
dismiss_zellij_popups
run_shell_command "printf \"\\nBP_E2E_OVERLAY:%s\\n\" \"\$PWD\""
wait_for_screen "BP_E2E_OVERLAY:$PEER" workspace-overlay-peer 10

run_tui_command ':worktree create e2e-created --base main'
wait_for_screen 'approval binds to this exact Worktrunk command' worktree-create-review 30
assert_screen_has 'repository' worktree-create-repository
assert_screen_has 'mutation' worktree-create-mutation
assert_screen_has 'project hooks' worktree-create-hooks
assert_screen_has ':approve' worktree-create-approve
assert_screen_has '--create e2e-created' worktree-create-command
assert_screen_lacks '--force' worktree-create-no-force
assert_screen_lacks '--clobber' worktree-create-no-clobber
run_tui_command ':approve'
wait_for_screen "Registered worktree $CREATED" worktree-create-approved 45
[ -d "$CREATED/.git" ] || [ -f "$CREATED/.git" ] ||
  fail_e2e 'approved Worktrunk create did not produce a Git worktree'
assert_screen_has 'e2e-created' worktree-created-tree

run_tui_command ':worktree remove'
wait_for_screen 'approval binds to this exact Worktrunk command' worktree-remove-review 30
assert_screen_has 'repository' worktree-remove-repository
assert_screen_has 'mutation' worktree-remove-mutation
assert_screen_has 'project hooks' worktree-remove-hooks
assert_screen_has ':approve' worktree-remove-approve
assert_screen_lacks '--force' worktree-remove-no-force
assert_screen_lacks '--force-delete' worktree-remove-no-force-delete
run_tui_command ':approve'
wait_for_screen 'Worktree removed through Worktrunk without force flags.' worktree-remove-approved 45
wait_for_path_absent "$CREATED" 20

# A host reboot can leave durable session records after the Zellij server is
# gone. Relaunch against the same registry and prove each recreated session
# receives its workspace environment and runs each auto-start service once.
run_tui_command ':quit'
wait_for_session_exit 15
ZELLIJ_SOCKET_DIR="$ZELLIJ_SOCKET_ROOT" "$ZELLIJ_BIN" kill-all-sessions -y
wait_for_no_zellij_session_sockets 'simulated reboot' 15

tmux -S "$TMUX_SOCKET" new-session -d -s "$TMUX_SESSION" -x 180 -y 52 \
  -c "$PRIMARY" "$BP_DEV"
wait_for_screen 'bp  blackpepper' reboot-startup 60
wait_for_screen 'primary' reboot-primary 60
wait_for_file_lines "$AUTO_SERVICE_LOG" 'workspace-env-ok' 2 20
wait_for_file_lines "$PEER_AUTO_SERVICE_LOG" 'workspace-env-ok' 2 20
sleep 0.5
wait_for_file_lines "$AUTO_SERVICE_LOG" 'workspace-env-ok' 2 1
wait_for_file_lines "$PEER_AUTO_SERVICE_LOG" 'workspace-env-ok' 2 1
send_enter
wait_for_terminal_mode reboot-attached 20
dismiss_zellij_popups
run_shell_command "printf \"\\nBP_E2E_REBOOT_SHELL:%s\\n\" \"\$BLACKPEPPER_E2E\""
wait_for_screen 'BP_E2E_REBOOT_SHELL:workspace-env-ok' reboot-shell-env 10

run_tui_command ':workspace switch primary'
wait_for_terminal_mode primary-before-terminate 20
dismiss_zellij_popups
run_tui_command ':workspace terminate'
wait_for_screen 'Zellij session terminated; the workspace folder was kept.' primary-terminated 20

# A branded backend name is deterministic for its exact Zellij generation.
# Recreate and terminate it twice to prove the exited registry row is reused
# instead of colliding with its unique workspace/backend/name identity.
for cycle in one two; do
  run_tui_command ':workspace switch primary'
  wait_for_terminal_mode "primary-reopen-$cycle" 20
  dismiss_zellij_popups
  run_shell_command "printf '\nBP_E2E_REOPEN_${cycle^^}\n'"
  wait_for_screen "BP_E2E_REOPEN_${cycle^^}" "primary-reopen-marker-$cycle" 10
  run_tui_command ':workspace terminate'
  wait_for_screen 'Zellij session terminated; the workspace folder was kept.' \
    "primary-reterminated-$cycle" 20
done

run_tui_command ':workspace switch peer'
wait_for_terminal_mode peer-before-terminate 20
dismiss_zellij_popups
run_tui_command ':workspace terminate'
wait_for_screen 'Zellij session terminated; the workspace folder was kept.' peer-terminated 20

run_tui_command ':host add e2e e2e-host'
wait_for_screen 'Added SSH host e2e; use :host connect e2e.' host-added 10
run_tui_command ':host disconnect e2e'
wait_for_screen 'Disconnected from e2e; sessions remain running.' host-disconnected 10
run_tui_command ':quit'
wait_for_session_exit 15

wait_for_no_zellij_session_sockets 'explicit workspace termination' 15

run_config_rejection invalid invalid-config.toml "unknown field \`unknown_v1_option\`"
run_config_rejection legacy legacy-tmux-config.toml 'Legacy [tmux] configuration found'

printf 'PASS: local TUI end-to-end acceptance (%s)\n' "$BP_VERSION"
printf 'PASS: Zellij %s, Worktrunk 0.72.0, PTY controls, workspaces, reboot restoration, services, ports, approvals, and config errors\n' \
  "$ZELLIJ_VERSION"
