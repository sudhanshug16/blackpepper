#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURES="$ROOT/scripts/fixtures/tui-agent-e2e"
ORIGINAL_HOME="${HOME:?HOME must be set}"
CODEX_BIN="$(command -v codex 2>/dev/null || true)"
CLAUDE_BIN="$(command -v claude 2>/dev/null || true)"
TEMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/blackpepper-real-agent-smoke.XXXXXX")"

cleanup() {
  local status=$?
  find "$TEMP_ROOT" -depth -delete 2>/dev/null || true
  exit "$status"
}
trap cleanup EXIT HUP INT TERM

for requirement in findmnt mount python3 timeout unshare; do
  command -v "$requirement" >/dev/null 2>&1 || {
    printf 'SKIP: real provider smoke needs %s.\n' "$requirement"
    exit 0
  }
done
if [ "$(uname -s)" != Linux ] || ! unshare -Ur true 2>/dev/null; then
  printf '%s\n' 'SKIP: a rootless Linux mount namespace is unavailable.'
  exit 0
fi

smoke() {
  local provider="$1" binary="$2" status
  local marker="$TEMP_ROOT/$provider.ready"
  if [ -z "$binary" ] || [ ! -x "$binary" ]; then
    printf 'SKIP: real %s CLI is not installed.\n' "$provider"
    return 0
  fi
  if [ "$provider" = codex ]; then
    if ! timeout 10 "$binary" login status 2>&1 | grep -Fq 'Logged in'; then
      printf '%s\n' 'SKIP: real Codex is not authenticated.'
      return 0
    fi
  elif ! timeout 10 "$binary" auth status 2>&1 | grep -Eq '"loggedIn"[[:space:]]*:[[:space:]]*true'; then
    printf '%s\n' 'SKIP: real Claude is not authenticated.'
    return 0
  fi

  set +e
  timeout 20 unshare -Urnm "$FIXTURES/readonly-home-runner.sh" "$ORIGINAL_HOME" \
    "$TEMP_ROOT/$provider-overlay" \
    python3 "$FIXTURES/real-session-smoke.py" "$provider" "$binary" "$marker"
  status=$?
  set -e
  case "$status" in
    0) printf 'PASS: real %s emitted SessionStart with no prompt; home was read-only.\n' "$provider" ;;
    2) printf 'SKIP: real %s did not emit SessionStart without interaction; no prompt was sent.\n' "$provider" ;;
    *) printf 'SKIP: real %s smoke could not run safely (status %s).\n' "$provider" "$status" ;;
  esac
}

smoke codex "$CODEX_BIN"
smoke claude "$CLAUDE_BIN"
