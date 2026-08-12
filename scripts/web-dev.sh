#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)"
TTYD_VERSION="1.7.7"
TTYD_SYSTEM_VERSION_OUTPUT="ttyd version 1.7.7"
TTYD_MANAGED_VERSION_OUTPUT="ttyd version 1.7.7-40e79c7"
TTYD_RELEASE_BASE_URL="${BLACKPEPPER_WEB_TTYD_RELEASE_BASE_URL:-https://github.com/tsl0922/ttyd/releases/download/${TTYD_VERSION}}"
TTYD_CACHE_ROOT="${BLACKPEPPER_WEB_TTYD_CACHE_DIR:-${ROOT}/target/dev-tools/ttyd/${TTYD_VERSION}}"
TTYD_INDEX_SOURCE="${BLACKPEPPER_WEB_TTYD_INDEX_SOURCE:-}"
CLIPBOARD_BRIDGE="$ROOT/scripts/fixtures/web-dev/clipboard-bridge.html"
PORT="${BLACKPEPPER_WEB_PORT:-7681}"
SKIP_BUILD=0

log() { printf '%s\n' "$*"; }
die() { log "Error: $*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Usage: scripts/web-dev.sh [--port PORT] [--skip-build]

Build and run bp-dev in a one-client, loopback-only browser terminal.

  --port PORT    Fixed loopback port (default: 7681)
  --skip-build   Use the currently installed bp-dev without running setup.sh
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --port)
      [ "$#" -ge 2 ] || die '--port requires a value'
      PORT="$2"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *) die "unexpected argument: $1" ;;
  esac
done

case "$PORT" in
  ''|*[!0-9]*) die 'port must be an integer from 1 through 65535' ;;
esac
if [ "$PORT" -lt 1 ] || [ "$PORT" -gt 65535 ]; then
  die 'port must be an integer from 1 through 65535'
fi

case "$TTYD_CACHE_ROOT" in
  /*) ;;
  *) die "ttyd cache path must be absolute: $TTYD_CACHE_ROOT" ;;
esac

# shellcheck source=scripts/fixtures/web-dev/harness-lib.sh
source "$ROOT/scripts/fixtures/web-dev/harness-lib.sh"

if [ "$SKIP_BUILD" -eq 0 ]; then
  "$ROOT/scripts/setup.sh"
fi

DEV_INSTALL_DIR="${DEV_INSTALL_DIR:-${HOME}/.local/bin}"
case "$DEV_INSTALL_DIR" in
  /*) ;;
  *) die "DEV_INSTALL_DIR must be absolute: $DEV_INSTALL_DIR" ;;
esac
CURRENT_BUNDLE="$DEV_INSTALL_DIR/.blackpepper-dev/current"
[ -d "$CURRENT_BUNDLE" ] ||
  die "bp-dev is not installed at $DEV_INSTALL_DIR/bp-dev; run scripts/setup.sh"
CURRENT_BUNDLE="$(CDPATH='' cd -P -- "$CURRENT_BUNDLE" && pwd)"
BP_DEV="$CURRENT_BUNDLE/bp-dev"
if [ ! -f "$BP_DEV" ] || [ -L "$BP_DEV" ] || [ ! -x "$BP_DEV" ]; then
  die "the current bp-dev bundle has no regular executable: $BP_DEV"
fi

TTYD="$(ensure_ttyd)"
BROWSER_INDEX="$(ensure_browser_index "$TTYD")"
CAPABILITY="$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
case "$CAPABILITY" in
  *[!0-9a-f]*|'') die 'could not generate the browser capability path' ;;
esac
[ "${#CAPABILITY}" -eq 32 ] || die 'could not generate a 128-bit browser capability path'
AUTH_SECRET="$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
case "$AUTH_SECRET" in
  *[!0-9a-f]*|'') die 'could not generate the browser authentication secret' ;;
esac
[ "${#AUTH_SECRET}" -eq 32 ] || die 'could not generate a 128-bit browser authentication secret'
[ "$AUTH_SECRET" != "$CAPABILITY" ] || die 'browser authentication entropy unexpectedly repeated'
BASE_PATH="/bp-dev-$CAPABILITY"

log "Blackpepper browser terminal: http://bp:${AUTH_SECRET}@127.0.0.1:${PORT}${BASE_PATH}/"
log 'Loopback only. The first browser client owns this one-shot PTY.'
log 'Zellij copy uses the browser clipboard when permitted; otherwise click the visible Copy button.'
log 'Close any running bp-dev first; production bp may remain open.'

exec "$TTYD" \
  -i 127.0.0.1 \
  -p "$PORT" \
  -W \
  -O \
  -m 1 \
  -o \
  -c "bp:$AUTH_SECRET" \
  -d 3 \
  -T xterm-256color \
  -b "$BASE_PATH" \
  -w "$ROOT" \
  -I "$BROWSER_INDEX" \
  -t allowProposedApi=true \
  -t rendererType=dom \
  -t screenReaderMode=true \
  -t disableReconnect=true \
  -t disableLeaveAlert=true \
  -t enableZmodem=false \
  -t enableTrzsz=false \
  -t enableSixel=false \
  "$BP_DEV"
