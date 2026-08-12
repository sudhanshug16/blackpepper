#!/usr/bin/env bash
set -euo pipefail

ORIGINAL_HOME="${HOME:?HOME must be set}"
# Zellij appends a substantial session/socket suffix and Linux limits Unix
# socket paths to 107 bytes. Keep this isolated root intentionally short.
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/bpse.XXXXXX")"
TMUX_SOCKET="$TEST_ROOT/tmux.sock"
SSH_LOG="$TEST_ROOT/ssh-argv.log"
SSHD_LOG="$TEST_ROOT/sshd.log"
SSHD_PID=''
LISTENER_PID=''
TMUX_SESSION=''
FAILED=1

SCRIPT_DIR="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
FIXTURE_DIR="$SCRIPT_DIR/fixtures/tui-ssh-e2e"
# shellcheck source=scripts/fixtures/tui-ssh-e2e/lifecycle-lib.sh
source "$FIXTURE_DIR/lifecycle-lib.sh"
# shellcheck source=scripts/fixtures/tui-ssh-e2e/tui-driver-lib.sh
source "$FIXTURE_DIR/tui-driver-lib.sh"
trap cleanup EXIT HUP INT TERM

for required in bash curl git nc python3 readlink sha256sum ssh ssh-keygen tmux; do
  require_command "$required"
done

SSHD="${BLACKPEPPER_E2E_SSHD:-/usr/sbin/sshd}"
[ -x "$SSHD" ] || fail "OpenSSH server is required at $SSHD"

BP_BINARY="$(resolve_bp_binary)"
BP_HOST_BINARY="$(dirname "$BP_BINARY")/bp-host"
[ -x "$BP_HOST_BINARY" ] || fail "bp-host must be beside the tested client: $BP_HOST_BINARY"

ZELLIJ_CACHE_RELATIVE='blackpepper/sidecars/zellij/0.44.3/x86_64-unknown-linux-musl'
ZELLIJ_ARCHIVE_SHA256='0f7c346788627f506c0a28296517768633cff24fc822a739f8264b640ecad751'
ZELLIJ_BINARY_SHA256='397481870c4fc3bae646cd7613cde3a1cebdc204558a6cb9a7c603d4c852fc90'
ZELLIJ_SOURCE_DIR="${BLACKPEPPER_E2E_ZELLIJ_SEED:-$ORIGINAL_HOME/.local/share/$ZELLIJ_CACHE_RELATIVE}"
ZELLIJ_SOURCE_BINARY="$ZELLIJ_SOURCE_DIR/zellij"
ZELLIJ_SOURCE_ARCHIVE="$ZELLIJ_SOURCE_DIR/zellij-x86_64-unknown-linux-musl.tar.gz"
[ -x "$ZELLIJ_SOURCE_BINARY" ] || fail "verified Zellij 0.44.3 cache is missing: $ZELLIJ_SOURCE_BINARY"
[ -f "$ZELLIJ_SOURCE_ARCHIVE" ] || fail "verified Zellij archive is missing: $ZELLIJ_SOURCE_ARCHIVE"
[ "$("$ZELLIJ_SOURCE_BINARY" --version)" = 'zellij 0.44.3' ] ||
  fail "Zellij seed is not version 0.44.3: $ZELLIJ_SOURCE_BINARY"
[ "$(sha256sum "$ZELLIJ_SOURCE_ARCHIVE" | awk '{print $1}')" = "$ZELLIJ_ARCHIVE_SHA256" ] ||
  fail "Zellij seed archive checksum is not pinned 0.44.3: $ZELLIJ_SOURCE_ARCHIVE"
[ "$(sha256sum "$ZELLIJ_SOURCE_BINARY" | awk '{print $1}')" = "$ZELLIJ_BINARY_SHA256" ] ||
  fail "Zellij seed binary checksum is not pinned 0.44.3: $ZELLIJ_SOURCE_BINARY"

SSHD_PORT="$(choose_port)"
REMOTE_PORT="$(choose_port)"
while [ "$REMOTE_PORT" = "$SSHD_PORT" ]; do
  REMOTE_PORT="$(choose_port)"
done

install -d -m 0700 \
  "$TEST_ROOT/sshd" \
  "$TEST_ROOT/bin" \
  "$TEST_ROOT/remote/config" \
  "$TEST_ROOT/remote/config/zellij" \
  "$TEST_ROOT/remote/cache" \
  "$TEST_ROOT/remote/data" \
  "$TEST_ROOT/remote/state" \
  "$TEST_ROOT/remote/runtime" \
  "$TEST_ROOT/remote/workspace" \
  "$TEST_ROOT/empty-config"

cat > "$TEST_ROOT/remote/config/zellij/config.kdl" <<'EOF'
show_startup_tips false
show_release_notes false
EOF
chmod 0600 "$TEST_ROOT/remote/config/zellij/config.kdl"

REMOTE_WORKSPACE="$TEST_ROOT/remote/workspace"
printf '%s\n' 'blackpepper-ssh-forward-ok' > "$REMOTE_WORKSPACE/marker.txt"
git -C "$REMOTE_WORKSPACE" init -q
git -C "$REMOTE_WORKSPACE" config user.name 'Blackpepper E2E'
git -C "$REMOTE_WORKSPACE" config user.email 'blackpepper-e2e@example.invalid'
git -C "$REMOTE_WORKSPACE" add marker.txt
git -C "$REMOTE_WORKSPACE" commit -qm 'test: seed remote workspace'

ssh-keygen -q -t ed25519 -N '' -f "$TEST_ROOT/sshd/host-key"
ssh-keygen -q -t ed25519 -N '' -f "$TEST_ROOT/sshd/client-key"
cp "$TEST_ROOT/sshd/client-key.pub" "$TEST_ROOT/sshd/authorized_keys"
chmod 0600 "$TEST_ROOT/sshd/authorized_keys"

cat > "$TEST_ROOT/sshd/sshd_config" <<EOF
Port $SSHD_PORT
ListenAddress 127.0.0.1
HostKey $TEST_ROOT/sshd/host-key
PidFile $TEST_ROOT/sshd/sshd.pid
AuthorizedKeysFile $TEST_ROOT/sshd/authorized_keys
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
PermitRootLogin no
UsePAM no
AllowUsers $(id -un)
StrictModes no
UseDNS no
PrintMotd no
LogLevel VERBOSE
AcceptEnv ZELLIJ_CONFIG_DIR XDG_CONFIG_HOME XDG_CACHE_HOME XDG_DATA_HOME XDG_STATE_HOME XDG_RUNTIME_DIR
EOF

SSH_CONFIG="$TEST_ROOT/ssh_config"
KNOWN_HOSTS="$TEST_ROOT/known_hosts"
cat > "$SSH_CONFIG" <<EOF
Host bp-e2e-alias
    HostName 127.0.0.1
    Port $SSHD_PORT
    User $(id -un)
    IdentityFile $TEST_ROOT/sshd/client-key
    IdentitiesOnly yes
    PreferredAuthentications publickey
    PasswordAuthentication no
    StrictHostKeyChecking ask
    UserKnownHostsFile $KNOWN_HOSTS
    GlobalKnownHostsFile /dev/null
    HashKnownHosts yes
    LogLevel ERROR
    SetEnv ZELLIJ_CONFIG_DIR=$TEST_ROOT/remote/config/zellij XDG_CONFIG_HOME=$TEST_ROOT/remote/config XDG_CACHE_HOME=$TEST_ROOT/remote/cache XDG_DATA_HOME=$TEST_ROOT/remote/data XDG_STATE_HOME=$TEST_ROOT/remote/state XDG_RUNTIME_DIR=$TEST_ROOT/remote/runtime
EOF
chmod 0600 "$SSH_CONFIG"

cat > "$TEST_ROOT/bin/ssh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
: "${BLACKPEPPER_E2E_SSH_CONFIG:?}"
: "${BLACKPEPPER_E2E_SSH_LOG:?}"
printf -v encoded ' <%q>' "$@"
printf '%s%s\n' "${EPOCHREALTIME:-0}" "$encoded" >> "$BLACKPEPPER_E2E_SSH_LOG"
exec /usr/bin/ssh -F "$BLACKPEPPER_E2E_SSH_CONFIG" "$@"
EOF
chmod 0700 "$TEST_ROOT/bin/ssh"
export BLACKPEPPER_E2E_SSH_CONFIG="$SSH_CONFIG"
export BLACKPEPPER_E2E_SSH_LOG="$SSH_LOG"

"$SSHD" -t -f "$TEST_ROOT/sshd/sshd_config"
"$SSHD" -D -e -f "$TEST_ROOT/sshd/sshd_config" > "$SSHD_LOG" 2>&1 &
SSHD_PID=$!
wait_until 'the temporary sshd listener' 10 tcp_port_open 127.0.0.1 "$SSHD_PORT"
[ ! -e "$KNOWN_HOSTS" ] || fail 'temporary known_hosts existed before first connection'

prepare_tui_fixtures

start_client client-a
send_command ':host import'
wait_until 'SSH config import preview' 15 screen_has 'SSH import preview'
screen_has 'bp-e2e-alias' || fail 'host import omitted the literal test alias'
save_screen client-a-import
tmux -S "$TMUX_SOCKET" send-keys -t "$TMUX_SESSION:0.0" Escape
wait_until 'SSH import preview close' 10 screen_lacks 'SSH import preview'

send_command ':host add lab bp-e2e-alias'
wait_until 'host registration' 10 screen_has 'Added SSH host lab'
send_command ':host connect lab'
wait_until 'first-use host-key prompt' 15 screen_has 'Are you sure you want to continue connecting'
screen_has 'OpenSSH owns authentication' ||
  fail 'authentication surface did not identify OpenSSH as the authority'
screen_has 'Blackpepper stores no credentials' ||
  fail 'authentication surface omitted the credential-storage boundary'
HOST_FINGERPRINT="$(ssh-keygen -lf "$TEST_ROOT/sshd/host-key.pub" -E sha256 | awk '{print $2}')"
screen_has "$HOST_FINGERPRINT" || fail 'SSH host-key prompt omitted the server fingerprint'
[ ! -e "$KNOWN_HOSTS" ] || fail 'known_hosts changed before the prompt was confirmed'
save_screen client-a-host-key
send_literal 'yes'
send_enter
wait_until 'first SSH connection' 60 screen_has 'SSH connected; restored'
[ -f "$KNOWN_HOSTS" ] || fail 'confirmed host key was not written to the isolated known_hosts file'
[ "$(stat -c '%a' "$KNOWN_HOSTS")" = 600 ] || fail 'isolated known_hosts permissions are not 0600'
[ "$(wc -l < "$KNOWN_HOSTS")" -eq 1 ] || fail 'isolated known_hosts should contain exactly one host key'
grep -Fq '|1|' "$KNOWN_HOSTS" || fail 'isolated known_hosts entry was not hashed'
ssh-keygen -F "[127.0.0.1]:$SSHD_PORT" -f "$KNOWN_HOSTS" >/dev/null ||
  fail 'isolated known_hosts does not resolve the temporary SSH endpoint'
save_screen client-a-connected

send_command ":workspace add $REMOTE_WORKSPACE"
wait_until 'remote workspace attach' 60 screen_is_terminal
ensure_work
wait_for_terminal_ready 'initial remote shell' 15
send_literal "printf 'BP_REMOTE_PTY_A:%s\\n' \"\$PWD\""
send_enter
wait_until 'remote Zellij PTY output' 20 screen_has "BP_REMOTE_PTY_A:$REMOTE_WORKSPACE"
save_screen client-a-remote-pty
assert_remote_sidecars

(
  cd "$REMOTE_WORKSPACE"
  exec python3 -m http.server "$REMOTE_PORT" --bind 127.0.0.1 > "$TEST_ROOT/listener.log" 2>&1
) &
LISTENER_PID=$!
wait_until 'remote workspace HTTP listener' 10 curl -fs --max-time 1 "http://127.0.0.1:$REMOTE_PORT/marker.txt" -o /dev/null

send_command ':ports'
wait_until 'workspace-attributed remote port' 20 screen_has "127.0.0.1:$REMOTE_PORT"
screen_matches "127\\.0\\.0\\.1:$REMOTE_PORT.*(python3|python)" ||
  fail 'remote listener was not attributed to its workspace process'
save_screen client-a-ports

send_command ":forward $REMOTE_PORT"
wait_until 'active SSH port forward' 20 screen_has 'Forward active: http://127.0.0.1:'
FORWARD_URL="$(capture_screen | grep -Eo 'http://127\.0\.0\.1:[0-9]+' | head -n 1)"
[ -n "$FORWARD_URL" ] || fail 'could not read the local forward mapping from the UI'
LOCAL_PORT="${FORWARD_URL##*:}"
[ "$LOCAL_PORT" != "$REMOTE_PORT" ] ||
  fail 'occupied same-number local port was reused instead of selecting a free port'
[ "$(curl -fsS --max-time 2 "$FORWARD_URL/marker.txt")" = 'blackpepper-ssh-forward-ok' ] ||
  fail 'SSH forward did not carry HTTP traffic to the remote workspace listener'
save_screen client-a-forward

ensure_work
send_literal "printf 'BP_REMOTE_PTY_CONCURRENT\\n'"
send_enter
wait_until 'PTY output while the forward is active' 15 screen_has 'BP_REMOTE_PTY_CONCURRENT'
[ "$(curl -fsS --max-time 2 "$FORWARD_URL/marker.txt")" = 'blackpepper-ssh-forward-ok' ] ||
  fail 'SSH forward stopped while the attached PTY remained active'

send_command ':host disconnect lab'
wait_until 'SSH disconnect' 15 screen_has 'Disconnected from lab; sessions remain running.'
wait_until 'forward closure on disconnect' 15 bash -c "! curl -fsS --max-time 0.3 '$FORWARD_URL/marker.txt' >/dev/null 2>&1"
save_screen client-a-disconnected

send_command ':host connect lab'
wait_until 'SSH reconnect' 60 screen_has 'SSH connected; restored'
[ "$(curl -fsS --max-time 2 "$FORWARD_URL/marker.txt")" = 'blackpepper-ssh-forward-ok' ] ||
  fail 'SSH reconnect did not restore the original local forward port'
send_command ':workspace switch workspace'
wait_until 'reattach to persistent remote Zellij session' 60 screen_is_terminal
ensure_work
wait_for_terminal_ready 'reattached remote shell' 15
send_literal "printf 'BP_REMOTE_PTY_REATTACHED\\n'"
send_enter
wait_until 'remote PTY after reconnect' 15 screen_has 'BP_REMOTE_PTY_REATTACHED'

send_command ":forward cancel $REMOTE_PORT"
wait_until 'forward cancellation' 15 screen_has "Cancelled forward for remote 127.0.0.1:$REMOTE_PORT."
wait_until 'closed cancelled forward' 15 bash -c "! curl -fsS --max-time 0.3 '$FORWARD_URL/marker.txt' >/dev/null 2>&1"
save_screen client-a-cancelled

REMOTE_REGISTRY="$TEST_ROOT/remote/state/blackpepper/host-registry.sqlite3"
[ -f "$REMOTE_REGISTRY" ] || fail 'remote registry was not written inside isolated XDG state'
[ "$(stat -c '%a' "$TEST_ROOT/remote/state/blackpepper")" = 700 ] ||
  fail 'remote Blackpepper state directory permissions are not 0700'
[ "$(stat -c '%a' "$REMOTE_REGISTRY")" = 600 ] || fail 'remote registry permissions are not 0600'
assert_registry "$REMOTE_REGISTRY" "$REMOTE_WORKSPACE" 'running'

grep -Fq 'ControlMaster=yes' "$SSH_LOG" || fail 'real SSH master was not launched in foreground master mode'
for option in ControlMaster=no ProxyJump=none ProxyCommand=false CanonicalizeHostname=no BatchMode=yes; do
  grep -Fq "$option" "$SSH_LOG" || fail "SSH child channels omitted fail-closed option $option"
done
grep -Fq '<-O> <forward>' "$SSH_LOG" || fail 'SSH mux did not receive the forward control operation'
grep -Fq '<-O> <cancel>' "$SSH_LOG" || fail 'SSH mux did not receive the cancel control operation'

stop_client

start_client client-b
send_command ':host import'
wait_until 'second client SSH import preview' 15 screen_has 'SSH import preview'
screen_has 'bp-e2e-alias' || fail 'second client host import omitted the literal test alias'
tmux -S "$TMUX_SOCKET" send-keys -t "$TMUX_SESSION:0.0" Escape
wait_until 'second client SSH import preview close' 10 screen_lacks 'SSH import preview'
send_command ':host add lab bp-e2e-alias'
wait_until 'second client host registration' 10 screen_has 'Added SSH host lab'
send_command ':host connect lab'
wait_until 'second client SSH connection' 60 screen_has 'SSH connected; restored'
wait_until 'host-side remote workspace discovery' 20 screen_has 'workspace'
save_screen client-b-discovery

send_command ':workspace switch workspace'
wait_until 'second-client session attach' 60 screen_is_terminal
ensure_work
wait_for_terminal_ready 'second-client remote shell' 15
send_literal "printf 'BP_REMOTE_PTY_CLIENT_B\\n'"
send_enter
wait_until 'second-client persistent PTY output' 15 screen_has 'BP_REMOTE_PTY_CLIENT_B'
save_screen client-b-attached

send_command ':workspace terminate'
wait_until 'explicit remote session termination' 30 screen_has 'Zellij session terminated; the workspace folder was kept.'
assert_registry "$REMOTE_REGISTRY" "$REMOTE_WORKSPACE" 'exited'
send_command ':host disconnect lab'
wait_until 'second-client SSH disconnect' 15 screen_has 'Disconnected from lab; sessions remain running.'
stop_client

NETWORK_CONNECTIONS="$(grep -c '^Accepted publickey ' "$SSHD_LOG" || true)"
[ "$NETWORK_CONNECTIONS" -eq 3 ] ||
  fail "expected exactly three SSH network connections (initial, reconnect, second client), got $NETWORK_CONNECTIONS; a mux child may have connected directly"

while IFS= read -r socket; do
  [ -n "$socket" ] || continue
  [ ! -e "$socket" ] || fail "SSH control socket remained after client exit: $socket"
done < <(grep -Eo '/tmp/bp-ssh-[^ /]+/c' "$SSH_LOG" | sort -u)

if kill -0 "$LISTENER_PID" 2>/dev/null; then
  kill "$LISTENER_PID"
  wait "$LISTENER_PID" 2>/dev/null || true
fi
LISTENER_PID=''

printf '%s\n' 'Blackpepper real SSH TUI E2E passed:'
printf '  client: %s\n' "$BP_BINARY"
printf '  host key: first-use prompt accepted into isolated hashed known_hosts\n'
printf '  remote: registry/helper/Zellij state stayed in temporary XDG roots\n'
printf '  sidecar: %s\n' "$(sidecar_mode)"
printf '  PTY: initial attach, reconnect attach, and second-client discovery passed\n'
printf '  transport: mux exec, port discovery, forward, reconnect, and cancel passed\n'

FAILED=0
