#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)"
DEV_TARGET_DIR="${BLACKPEPPER_DEV_TARGET_DIR:-${ROOT}/target}"
CARGO_BIN="${CARGO:-$(command -v cargo 2>/dev/null || true)}"
RUSTC_BIN="${RUSTC:-$(command -v rustc 2>/dev/null || true)}"
STRIP_BIN="${STRIP:-$(command -v strip 2>/dev/null || true)}"
[ -n "$CARGO_BIN" ] || CARGO_BIN="${HOME}/.cargo/bin/cargo"
[ -n "$RUSTC_BIN" ] || RUSTC_BIN="${HOME}/.cargo/bin/rustc"
[ -x "$CARGO_BIN" ] || { printf 'Error: cargo is not executable: %s\n' "$CARGO_BIN" >&2; exit 1; }
[ -x "$RUSTC_BIN" ] || { printf 'Error: rustc is not executable: %s\n' "$RUSTC_BIN" >&2; exit 1; }
if [ -z "$STRIP_BIN" ] || [ ! -x "$STRIP_BIN" ]; then
  printf '%s\n' 'Error: strip is required for compact source-run bundles.' >&2
  exit 1
fi
HOST_TARGET="$($RUSTC_BIN -vV | sed -n 's/^host: //p')"
[ -n "$HOST_TARGET" ] || { printf '%s\n' 'Error: could not determine Rust host target.' >&2; exit 1; }
export BLACKPEPPER_DEV_WATCH_CARGO="${BLACKPEPPER_DEV_WATCH_CARGO:-$CARGO_BIN}"
export BLACKPEPPER_DEV_WATCH_HOST_TARGET="${BLACKPEPPER_DEV_WATCH_HOST_TARGET:-$HOST_TARGET}"
export BLACKPEPPER_DEV_WATCH_STRIP="${BLACKPEPPER_DEV_WATCH_STRIP:-$STRIP_BIN}"

"$CARGO_BIN" build --manifest-path "$ROOT/Cargo.toml" \
  --target-dir "$DEV_TARGET_DIR" --target "$HOST_TARGET" \
  -p blackpepper-dev-watch --bin bp-dev-watch
exec "$DEV_TARGET_DIR/$HOST_TARGET/debug/bp-dev-watch" "$@"
