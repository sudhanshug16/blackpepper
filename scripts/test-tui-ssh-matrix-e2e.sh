#!/usr/bin/env bash
set -euo pipefail

ORIGINAL_HOME="${HOME:?HOME must be set}"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/bpsm.XXXXXX")"
TMUX_SOCKET="$TEST_ROOT/tmux.sock"
SSH_LOG="$TEST_ROOT/ssh-argv.log"
JUMP_LOG="$TEST_ROOT/jump-sshd.log"
TARGET_LOG="$TEST_ROOT/target-sshd.log"
SSHD_LOG="$TARGET_LOG"
JUMP_PID=''
TARGET_PID=''
LISTENER_PID=''
TMUX_SESSION=''
FAILED=1

SCRIPT_DIR="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
FIXTURE_DIR="$SCRIPT_DIR/fixtures/tui-ssh-e2e"
# shellcheck source=scripts/fixtures/tui-ssh-e2e/lifecycle-lib.sh
source "$FIXTURE_DIR/lifecycle-lib.sh"
# shellcheck source=scripts/fixtures/tui-ssh-e2e/tui-driver-lib.sh
source "$FIXTURE_DIR/tui-driver-lib.sh"
# shellcheck source=scripts/fixtures/tui-ssh-matrix/matrix-lib.sh
source "$SCRIPT_DIR/fixtures/tui-ssh-matrix/matrix-lib.sh"
trap matrix_cleanup EXIT HUP INT TERM

for required in bash git nc python3 readlink sha256sum ssh ssh-keygen tmux; do
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
[ -x "$ZELLIJ_SOURCE_BINARY" ] || fail "verified Zellij cache is missing: $ZELLIJ_SOURCE_BINARY"
[ -f "$ZELLIJ_SOURCE_ARCHIVE" ] || fail "verified Zellij archive is missing: $ZELLIJ_SOURCE_ARCHIVE"
[ "$("$ZELLIJ_SOURCE_BINARY" --version)" = 'zellij 0.44.3' ] ||
  fail "Zellij seed is not version 0.44.3: $ZELLIJ_SOURCE_BINARY"
[ "$(sha256sum "$ZELLIJ_SOURCE_ARCHIVE" | awk '{print $1}')" = "$ZELLIJ_ARCHIVE_SHA256" ] ||
  fail 'Zellij seed archive checksum does not match the pinned release'
[ "$(sha256sum "$ZELLIJ_SOURCE_BINARY" | awk '{print $1}')" = "$ZELLIJ_BINARY_SHA256" ] ||
  fail 'Zellij seed binary checksum does not match the pinned release'

JUMP_PORT="$(choose_port)"
TARGET_PORT="$(choose_port)"
while [ "$TARGET_PORT" = "$JUMP_PORT" ]; do
  TARGET_PORT="$(choose_port)"
done

install -d -m 0700 \
  "$TEST_ROOT/sshd" \
  "$TEST_ROOT/bin" \
  "$TEST_ROOT/remote/config" \
  "$TEST_ROOT/remote/data" \
  "$TEST_ROOT/remote/state" \
  "$TEST_ROOT/remote/runtime" \
  "$TEST_ROOT/remote/workspace" \
  "$TEST_ROOT/empty-config"

REMOTE_WORKSPACE="$TEST_ROOT/remote/workspace"
printf '%s\n' 'blackpepper-proxyjump-ok' > "$REMOTE_WORKSPACE/marker.txt"
git -C "$REMOTE_WORKSPACE" init -q
git -C "$REMOTE_WORKSPACE" config user.name 'Blackpepper E2E'
git -C "$REMOTE_WORKSPACE" config user.email 'blackpepper-e2e@example.invalid'
git -C "$REMOTE_WORKSPACE" add marker.txt
git -C "$REMOTE_WORKSPACE" commit -qm 'test: seed ProxyJump workspace'

ssh-keygen -q -t ed25519 -N '' -f "$TEST_ROOT/sshd/jump-host-key"
ssh-keygen -q -t ed25519 -N '' -f "$TEST_ROOT/sshd/target-host-key"
ssh-keygen -q -t ed25519 -N 'matrix-secret' -f "$TEST_ROOT/sshd/client-key"
cp "$TEST_ROOT/sshd/client-key.pub" "$TEST_ROOT/sshd/authorized_keys"
chmod 0600 "$TEST_ROOT/sshd/authorized_keys"

JUMP_CONFIG="$TEST_ROOT/sshd/jump_config"
TARGET_CONFIG="$TEST_ROOT/sshd/target_config"
write_sshd_config "$JUMP_CONFIG" "$JUMP_PORT" "$TEST_ROOT/sshd/jump-host-key"
write_sshd_config "$TARGET_CONFIG" "$TARGET_PORT" "$TEST_ROOT/sshd/target-host-key"

SSH_CONFIG="$TEST_ROOT/ssh_config"
KNOWN_HOSTS="$TEST_ROOT/known_hosts"
{
  printf '%s\n' 'Host bp-jump-e2e'
  printf '%s\n' '    HostName 127.0.0.1'
  printf '    Port %s\n' "$JUMP_PORT"
  printf '    User %s\n' "$(id -un)"
  printf '    IdentityFile %s\n' "$TEST_ROOT/sshd/client-key"
  printf '%s\n' '    IdentitiesOnly yes'
  printf '%s\n' '    PreferredAuthentications publickey'
  printf '%s\n' '    StrictHostKeyChecking ask'
  printf '    UserKnownHostsFile %s\n' "$KNOWN_HOSTS"
  printf '%s\n' '    GlobalKnownHostsFile /dev/null'
  printf '%s\n' '    HashKnownHosts yes'
  printf '%s\n' '    LogLevel ERROR'
  printf '%s\n' 'Host bp-proxy-e2e'
  printf '%s\n' '    HostName 127.0.0.1'
  printf '    Port %s\n' "$TARGET_PORT"
  printf '    User %s\n' "$(id -un)"
  printf '%s\n' '    ProxyJump bp-jump-e2e'
  printf '    IdentityFile %s\n' "$TEST_ROOT/sshd/client-key"
  printf '%s\n' '    IdentitiesOnly yes'
  printf '%s\n' '    PreferredAuthentications publickey'
  printf '%s\n' '    StrictHostKeyChecking ask'
  printf '    UserKnownHostsFile %s\n' "$KNOWN_HOSTS"
  printf '%s\n' '    GlobalKnownHostsFile /dev/null'
  printf '%s\n' '    HashKnownHosts yes'
  printf '%s\n' '    LogLevel ERROR'
  printf '    SetEnv XDG_CONFIG_HOME=%s XDG_DATA_HOME=%s XDG_STATE_HOME=%s XDG_RUNTIME_DIR=%s\n' \
    "$TEST_ROOT/remote/config" "$TEST_ROOT/remote/data" \
    "$TEST_ROOT/remote/state" "$TEST_ROOT/remote/runtime"
} > "$SSH_CONFIG"
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

start_sshd "$JUMP_CONFIG" "$JUMP_LOG" JUMP_PID "$JUMP_PORT"
start_sshd "$TARGET_CONFIG" "$TARGET_LOG" TARGET_PID "$TARGET_PORT"
[ ! -e "$KNOWN_HOSTS" ] || fail 'isolated known_hosts existed before first connection'

prepare_tui_fixtures
start_client client-a
send_command ':host add proxy bp-proxy-e2e'
wait_until 'ProxyJump host registration' 10 screen_has 'Added SSH host proxy'
send_command ':host connect proxy'

JUMP_FINGERPRINT="$(ssh-keygen -lf "$TEST_ROOT/sshd/jump-host-key.pub" -E sha256 | awk '{print $2}')"
TARGET_FINGERPRINT="$(ssh-keygen -lf "$TEST_ROOT/sshd/target-host-key.pub" -E sha256 | awk '{print $2}')"
wait_until 'jump-host first-use prompt' 15 screen_has "$JUMP_FINGERPRINT"
send_literal 'yes'
send_enter
wait_until 'jump-host encrypted-key prompt' 15 screen_count_at_least 'Enter passphrase for key' 1
send_literal 'matrix-secret'
send_enter
wait_until 'target first-use prompt through ProxyJump' 15 screen_has "$TARGET_FINGERPRINT"
send_literal 'yes'
send_enter
wait_until 'target encrypted-key prompt' 15 screen_count_at_least 'Enter passphrase for key' 2
send_literal 'matrix-secret'
send_enter
wait_until 'ProxyJump SSH connection' 60 screen_has 'SSH connected; restored'
JUMP_NETWORK_AFTER_CONNECT="$(network_connections "$JUMP_LOG")"
TARGET_NETWORK_AFTER_CONNECT="$(network_connections "$TARGET_LOG")"

[ "$(wc -l < "$KNOWN_HOSTS")" -eq 2 ] ||
  fail 'ProxyJump first use should record exactly the jump and target host keys'
[ "$(stat -c '%a' "$KNOWN_HOSTS")" = 600 ] ||
  fail 'isolated ProxyJump known_hosts permissions are not 0600'
grep -Fq '|1|' "$KNOWN_HOSTS" || fail 'ProxyJump known_hosts entries were not hashed'
ssh-keygen -F "[127.0.0.1]:$JUMP_PORT" -f "$KNOWN_HOSTS" >/dev/null ||
  fail 'known_hosts omitted the jump endpoint'
ssh-keygen -F "[127.0.0.1]:$TARGET_PORT" -f "$KNOWN_HOSTS" >/dev/null ||
  fail 'known_hosts omitted the target endpoint'
KNOWN_HOSTS_BEFORE="$(sha256sum "$KNOWN_HOSTS" | awk '{print $1}')"

send_command ":workspace add $REMOTE_WORKSPACE"
wait_until 'remote PTY through ProxyJump' 60 screen_has 'WORK'
ensure_work
dismiss_zellij_tip
send_literal "printf 'BP_PROXYJUMP_PTY:%s\\n' \"\$PWD\""
send_enter
wait_until 'ProxyJump Zellij PTY output' 20 screen_has "BP_PROXYJUMP_PTY:$REMOTE_WORKSPACE"
assert_connection_counts 1 1 'muxed ProxyJump workspace operations opened extra network connections'
[ "$(network_connections "$JUMP_LOG")" -eq "$JUMP_NETWORK_AFTER_CONNECT" ] ||
  fail 'a mux child opened another TCP connection to the jump host'
[ "$(network_connections "$TARGET_LOG")" -eq "$TARGET_NETWORK_AFTER_CONNECT" ] ||
  fail 'a mux child opened another TCP connection to the target host'

for option in ControlMaster=no ProxyJump=none ProxyCommand=false CanonicalizeHostname=no BatchMode=yes; do
  grep -Fq "$option" "$SSH_LOG" || fail "SSH child channels omitted fail-closed option $option"
done
grep -Fq 'ControlMaster=yes' "$SSH_LOG" || fail 'ProxyJump master did not use foreground master mode'
assert_fail_closed_without_mux

send_command ':host disconnect proxy'
wait_until 'ProxyJump disconnect' 15 screen_has 'Disconnected from proxy; sessions remain running.'
stop_process "$TARGET_PID"
TARGET_PID=''
rm -f -- "$TEST_ROOT/sshd/target-host-key" "$TEST_ROOT/sshd/target-host-key.pub"
ssh-keygen -q -t ed25519 -N '' -f "$TEST_ROOT/sshd/target-host-key"
ROTATED_FINGERPRINT="$(ssh-keygen -lf "$TEST_ROOT/sshd/target-host-key.pub" -E sha256 | awk '{print $2}')"
[ "$ROTATED_FINGERPRINT" != "$TARGET_FINGERPRINT" ] || fail 'target host-key rotation did not change the fingerprint'
start_sshd "$TARGET_CONFIG" "$TARGET_LOG" TARGET_PID "$TARGET_PORT"

send_command ':host connect proxy'
wait_until 'reconnect jump-host passphrase prompt' 15 screen_count_at_least 'Enter passphrase for key' 1
send_literal 'matrix-secret'
send_enter
wait_until 'visible changed-host-key refusal' 20 \
  screen_has 'SSH host key changed. Verify the host before updating known_hosts and reconnecting.'
[ "$(sha256sum "$KNOWN_HOSTS" | awk '{print $1}')" = "$KNOWN_HOSTS_BEFORE" ] ||
  fail 'changed-host-key refusal modified known_hosts'
assert_connection_counts 2 1 'changed-host-key attempt authenticated unexpectedly'

stop_client

while IFS= read -r socket; do
  [ -n "$socket" ] || continue
  [ ! -e "$socket" ] || fail "SSH control socket remained after client exit: $socket"
done < <(grep -Eo '/tmp/bp-ssh-[^ /]+/c' "$SSH_LOG" | sort -u)

printf '%s\n' 'Blackpepper live SSH security matrix passed:'
printf '  ProxyJump: jump and target first-use keys accepted through one foreground master\n'
printf '  passphrase: encrypted key prompts traversed the Blackpepper authentication PTY\n'
printf '  mux: remote helper and Zellij PTY reused one network connection per sshd\n'
printf '  fail closed: a missing mux socket created no jump or target connection\n'
printf '  changed key: replacement was blocked, visible, and known_hosts stayed unchanged\n'

FAILED=0
