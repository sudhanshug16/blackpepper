#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURES="$ROOT/scripts/fixtures/tui-agent-e2e"
# shellcheck source=scripts/fixtures/tui-agent-e2e/harness-lib.sh
source "$FIXTURES/harness-lib.sh"
# shellcheck source=scripts/fixtures/tui-agent-e2e/scenario-lib.sh
source "$FIXTURES/scenario-lib.sh"

for requirement in git python3 readlink tmux timeout; do
  command -v "$requirement" >/dev/null 2>&1 || {
    printf 'FAIL: required command is unavailable: %s\n' "$requirement" >&2
    exit 1
  }
done
ZELLIJ_VERSION="$(python3 "$ROOT/scripts/fixtures/zellij_runtime.py" version)"

if [ "$(uname -s)" != Linux ]; then
  printf '%s\n' 'FAIL: the agent PTY acceptance harness currently requires Linux.' >&2
  exit 1
fi

resolve_client() {
  if [ -n "${BLACKPEPPER_AGENT_E2E_BP:-}" ]; then
    readlink -f "$BLACKPEPPER_AGENT_E2E_BP"
  else
    local launcher launcher_dir current
    launcher="$(command -v bp-dev 2>/dev/null || true)"
    [ -n "$launcher" ] || return 1
    launcher_dir="$(CDPATH='' cd -P -- "$(dirname "$launcher")" && pwd)"
    current="$launcher_dir/.blackpepper-dev/current"
    [ -d "$current" ] || return 1
    current="$(CDPATH='' cd -P -- "$current" && pwd)"
    printf '%s\n' "$current/bp-dev"
  fi
}

BP_BINARY="$(resolve_client)" || {
  printf '%s\n' 'FAIL: build bp + bp-host or install bp-dev before agent E2E.' >&2
  exit 1
}
BP_HOST="$(dirname "$BP_BINARY")/bp-host"
if [ ! -x "$BP_BINARY" ] || [ ! -x "$BP_HOST" ]; then
  printf 'FAIL: matching client/helper bundle is incomplete beside %s\n' "$BP_BINARY" >&2
  exit 1
fi
BP_VERSION="$($BP_BINARY --version 2>&1)"
HOST_VERSION="$($BP_HOST --version 2>&1)"
BUILD_ID="${BP_VERSION#blackpepper }"
if [ "$BP_VERSION" = "$BUILD_ID" ] || [ "$HOST_VERSION" != "bp-host $BUILD_ID" ]; then
  printf 'FAIL: client/helper build IDs differ: %s / %s\n' "$BP_VERSION" "$HOST_VERSION" >&2
  exit 1
fi
case "$BUILD_ID" in
  *-dev.*) AGENT_EVENT_DATABASE='agent-events-dev.sqlite3' ;;
  *) printf 'FAIL: expected a development build, got: %s\n' "$BP_VERSION" >&2; exit 1 ;;
esac

TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/blackpepper-agent-e2e.XXXXXX")"
ARTIFACTS="$TEST_ROOT/artifacts"
WORKSPACE="$TEST_ROOT/fixture"
TEMP_HOME="$TEST_ROOT/home"
PROVIDER_ROOT="$TEST_ROOT/provider"
TMUX_SOCKET="$TEST_ROOT/tmux.sock"
TMUX_SESSION='bp-agent-e2e'
ZELLIJ_SOCKET_ROOT="$TEST_ROOT/z"
E2E_DATA_HOME="${BLACKPEPPER_AGENT_E2E_DATA_HOME:-$ROOT/target/tui-local-e2e-data}"
ZELLIJ_CACHE_ROOT="$E2E_DATA_HOME/blackpepper/sidecars/zellij/$ZELLIJ_VERSION"
E2E_SECRET='BP_AGENT_E2E_SECRET_MUST_NEVER_PERSIST_9e7b51'
ZELLIJ_BIN=''

case "$E2E_DATA_HOME" in
  /*) ;;
  *) printf 'FAIL: agent E2E data home must be absolute: %s\n' "$E2E_DATA_HOME" >&2; exit 1 ;;
esac

trap cleanup_agent_e2e EXIT HUP INT TERM
install -d -m 0700 \
  "$ARTIFACTS" "$WORKSPACE/.blackpepper" "$TEMP_HOME" "$PROVIDER_ROOT" \
  "$TEST_ROOT/bin" "$TEST_ROOT/config" "$TEST_ROOT/state" "$TEST_ROOT/run" \
  "$TEST_ROOT/cache" "$ZELLIJ_SOCKET_ROOT" "$E2E_DATA_HOME"
install -m 0700 "$FIXTURES/provider-shim.py" "$TEST_ROOT/bin/provider-shim"
ln -s provider-shim "$TEST_ROOT/bin/codex"
ln -s provider-shim "$TEST_ROOT/bin/claude"
ln -s provider-shim "$TEST_ROOT/bin/opencode"

{
  printf '%s\n' '[workspace.env]'
  printf 'BLACKPEPPER_AGENT_E2E_ROOT = "%s"\n' "$PROVIDER_ROOT"
  printf 'BLACKPEPPER_AGENT_E2E_SECRET = "%s"\n' "$E2E_SECRET"
} > "$WORKSPACE/.blackpepper/config.toml"
printf '%s\n' '# Blackpepper agent acceptance fixture' > "$WORKSPACE/README.md"
git -C "$WORKSPACE" init --initial-branch=main --quiet
git -C "$WORKSPACE" config user.name 'Blackpepper E2E'
git -C "$WORKSPACE" config user.email 'blackpepper-e2e@example.invalid'
git -C "$WORKSPACE" add README.md .blackpepper/config.toml
git -C "$WORKSPACE" commit --quiet -m 'test: initialize agent fixture'

export HOME="$TEMP_HOME"
export XDG_CONFIG_HOME="$TEST_ROOT/config"
export XDG_STATE_HOME="$TEST_ROOT/state"
export XDG_RUNTIME_DIR="$TEST_ROOT/run"
export XDG_CACHE_HOME="$TEST_ROOT/cache"
export XDG_DATA_HOME="$E2E_DATA_HOME"
export BLACKPEPPER_E2E_ZELLIJ_SOCKET_DIR="$ZELLIJ_SOCKET_ROOT"
unset ZELLIJ_SOCKET_DIR
export BLACKPEPPER_AGENT_E2E_ROOT="$PROVIDER_ROOT"
export BLACKPEPPER_AGENT_E2E_SECRET="$E2E_SECRET"
export BLACKPEPPER_AGENT_E2E_DB="$AGENT_EVENT_DATABASE"
export PATH="$TEST_ROOT/bin:/usr/local/bin:/usr/bin:/bin"
export SHELL='/bin/bash'
export TERM='xterm-256color'
export LANG='C.UTF-8'
export LC_ALL='C.UTF-8'
unset OPENCODE_CONFIG_CONTENT

start_client
attach_workspace
for _attempt in $(seq 1 200); do
  ZELLIJ_BIN="$(find "$ZELLIJ_CACHE_ROOT" \
    -type f -name zellij -perm -u+x -print -quit 2>/dev/null || true)"
  [ -n "$ZELLIJ_BIN" ] && break
  sleep 0.1
done
if [ -z "$ZELLIJ_BIN" ] || [ "$($ZELLIJ_BIN --version)" != "zellij $ZELLIJ_VERSION" ]; then
  fail_agent_e2e "exact managed Zellij $ZELLIJ_VERSION was not available"
fi

for provider in codex claude opencode; do
  exercise_provider "$provider"
done

run_tui_command ':workspace terminate'
wait_for_screen 'Zellij session terminated; the workspace folder was kept.' terminated 30
stop_client

STATE_DIR="$TEST_ROOT/state/blackpepper"
[ "$(stat -c '%a' "$STATE_DIR")" = 700 ] || fail_agent_e2e 'agent state directory is not 0700'
[ "$(stat -c '%a' "$STATE_DIR/$AGENT_EVENT_DATABASE")" = 600 ] ||
  fail_agent_e2e 'agent event database is not 0600'
state_tool assert-redacted "$E2E_SECRET" || fail_agent_e2e 'final database redaction check failed'
if find "$STATE_DIR/integrations" -type f -print -quit 2>/dev/null | grep -q .; then
  fail_agent_e2e 'managed launch assets remained after all provider exits'
fi
for personal_config in \
  "$TEMP_HOME/.codex/config.toml" \
  "$TEMP_HOME/.claude/settings.json" \
  "$TEMP_HOME/.config/opencode/opencode.json"; do
  [ ! -e "$personal_config" ] || fail_agent_e2e "provider shim changed user config: $personal_config"
done

printf 'PASS: real PTY agent UX validated for Codex, Claude, and OpenCode (%s).\n' "$BUILD_ID"
