#!/usr/bin/env bash
set -euo pipefail
shopt -s dotglob nullglob

ROOT="$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)"
DEV_INSTALL_DIR="${DEV_INSTALL_DIR:-${HOME}/.local/bin}"
DEV_TARGET_DIR="${BLACKPEPPER_DEV_TARGET_DIR:-${ROOT}/target}"
DEV_DATA_ROOT="${BLACKPEPPER_DEV_DATA_DIR:-${XDG_DATA_HOME:-${HOME}/.local/share}/blackpepper/dev-builds}"
LINK_ROOT="$DEV_INSTALL_DIR/.blackpepper-dev"
MARKER_NAME=".blackpepper-dev-bundle"
MARKER_VERSION="blackpepper-dev-bundle-v1"
MODE="install"
PRUNE_NAME=""
STAGE_DIR=""
INDEX_FILE=""
LINK_TEMP=""
WRAPPER_TEMP=""
WRAPPER_BACKUP=""
WRAPPER_PUBLISHED=0
CURRENT_SWITCHED=0
OLD_CURRENT_PRESENT=0
OLD_CURRENT_TARGET=""
INSTALL_SUCCEEDED=0

log() { printf '%s\n' "$*"; }
die() { log "Error: $*" >&2; exit 1; }

replace_path() {
  local source="$1" destination="$2"
  case "$(uname -s)" in
    Darwin) mv -fh "$source" "$destination" ;;
    Linux) mv -Tf -- "$source" "$destination" ;;
    *) die "unsupported development client OS: $(uname -s)" ;;
  esac
}

write_wrapper() {
  local destination="$1" marker="${2:-yes}"
  {
    printf '%s\n' '#!/bin/sh'
    [ "$marker" = no ] || printf '%s\n' '# blackpepper-dev-launcher-v1'
    # These variables intentionally expand when the installed launcher runs.
    # shellcheck disable=SC2016
    printf '%s\n' \
      'set -eu' \
      'SELF="$0"' \
      'case "$SELF" in */*) ;; *) SELF=$(command -v "$SELF") ;; esac' \
      'SELF_DIR=$(CDPATH= cd -P "$(dirname "$SELF")" && pwd)' \
      'exec "$SELF_DIR/.blackpepper-dev/current/bp-dev" "$@"'
  } > "$destination"
  chmod 0755 "$destination"
}

# shellcheck source=scripts/dev-installer-bundles.sh
source "$ROOT/scripts/dev-installer-bundles.sh"

cleanup_files() {
  [ -z "$STAGE_DIR" ] || rm -rf "$STAGE_DIR"
  [ -z "$INDEX_FILE" ] || rm -f "$INDEX_FILE"
  [ -z "$LINK_TEMP" ] || rm -f "$LINK_TEMP"
  [ -z "$WRAPPER_TEMP" ] || rm -f "$WRAPPER_TEMP"
  [ -z "$WRAPPER_BACKUP" ] || rm -f "$WRAPPER_BACKUP"
}

fingerprint_source() {
  INDEX_FILE="$(mktemp "${TMPDIR:-/tmp}/blackpepper-index.XXXXXX")"
  rm -f "$INDEX_FILE"
  GIT_INDEX_FILE="$INDEX_FILE" git -C "$ROOT" read-tree HEAD
  GIT_INDEX_FILE="$INDEX_FILE" git -C "$ROOT" add -A -- .
  SOURCE_HASH_RESULT="$(GIT_INDEX_FILE="$INDEX_FILE" git -C "$ROOT" write-tree)"
  rm -f "$INDEX_FILE"
  INDEX_FILE=""
}

rollback() {
  local rollback_link
  set +e
  if [ "$CURRENT_SWITCHED" -eq 1 ]; then
    if [ "$OLD_CURRENT_PRESENT" -eq 1 ]; then
      rollback_link="$LINK_ROOT/.current-rollback.$$"
      ln -s "$OLD_CURRENT_TARGET" "$rollback_link" &&
        replace_path "$rollback_link" "$LINK_ROOT/current"
      rm -f "$rollback_link"
    else
      rm -f "$LINK_ROOT/current"
    fi
  fi
  if [ "$WRAPPER_PUBLISHED" -eq 1 ]; then
    if [ -n "$WRAPPER_BACKUP" ] && [ -f "$WRAPPER_BACKUP" ]; then
      replace_path "$WRAPPER_BACKUP" "$DEV_INSTALL_DIR/bp-dev"
      WRAPPER_BACKUP=""
    else
      rm -f "$DEV_INSTALL_DIR/bp-dev"
    fi
  fi
  set -e
}

on_exit() {
  local status=$?
  trap - EXIT HUP INT TERM
  if [ "$status" -ne 0 ] || [ "$INSTALL_SUCCEEDED" -ne 1 ]; then rollback; fi
  cleanup_files
  exit "$status"
}
trap on_exit EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

case "${1:-}" in
  '') ;;
  --list-builds) [ "$#" -eq 1 ] || die 'usage: setup.sh [--list-builds | --prune BUNDLE_NAME]'; MODE=list ;;
  --prune) [ "$#" -eq 2 ] || die 'usage: setup.sh --prune BUNDLE_NAME'; MODE=prune; PRUNE_NAME="$2" ;;
  *) die 'usage: setup.sh [--list-builds | --prune BUNDLE_NAME]' ;;
esac

for path in "$DEV_INSTALL_DIR" "$DEV_TARGET_DIR" "$DEV_DATA_ROOT"; do
  case "$path" in /*) ;; *) die "development install paths must be absolute: $path" ;; esac
done
[ ! -L "$DEV_DATA_ROOT" ] || die "refusing symbolic development data directory: $DEV_DATA_ROOT"
install -d -m 0700 "$DEV_DATA_ROOT"
chmod 0700 "$DEV_DATA_ROOT"

if [ "$MODE" = list ]; then list_bundles; INSTALL_SUCCEEDED=1; exit 0; fi
if [ "$MODE" = prune ]; then prune_bundle "$PRUNE_NAME"; INSTALL_SUCCEEDED=1; exit 0; fi

install -d -m 0755 "$DEV_INSTALL_DIR"
[ ! -L "$LINK_ROOT" ] || die "refusing symbolic development package directory: $LINK_ROOT"
install -d -m 0755 "$LINK_ROOT"
[ ! -e "$LINK_ROOT/current" ] || [ -L "$LINK_ROOT/current" ] ||
  die "refusing non-symbolic development current pointer: $LINK_ROOT/current"

WRAPPER_TEMP="$(mktemp "$DEV_INSTALL_DIR/.bp-dev.XXXXXX")"
write_wrapper "$WRAPPER_TEMP"
sh -n "$WRAPPER_TEMP"
if [ -e "$DEV_INSTALL_DIR/bp-dev" ] || [ -L "$DEV_INSTALL_DIR/bp-dev" ]; then
  if [ ! -f "$DEV_INSTALL_DIR/bp-dev" ] || [ -L "$DEV_INSTALL_DIR/bp-dev" ]; then
    die "refusing unrelated existing bp-dev: $DEV_INSTALL_DIR/bp-dev"
  fi
  LEGACY_WRAPPER="$(mktemp "$DEV_INSTALL_DIR/.bp-dev-legacy.XXXXXX")"
  write_wrapper "$LEGACY_WRAPPER" no
  if ! cmp -s "$DEV_INSTALL_DIR/bp-dev" "$WRAPPER_TEMP" &&
    ! cmp -s "$DEV_INSTALL_DIR/bp-dev" "$LEGACY_WRAPPER"; then
    rm -f "$LEGACY_WRAPPER"
    die "refusing unrelated existing bp-dev: $DEV_INSTALL_DIR/bp-dev"
  fi
  rm -f "$LEGACY_WRAPPER"
  WRAPPER_BACKUP="$(mktemp "$DEV_INSTALL_DIR/.bp-dev-backup.XXXXXX")"
  cp -p "$DEV_INSTALL_DIR/bp-dev" "$WRAPPER_BACKUP"
fi

PACKAGE_VERSION="$(sed -nE 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' \
  "$ROOT/crates/blackpepper/Cargo.toml" | head -n1)"
printf '%s\n' "$PACKAGE_VERSION" | grep -Eq '^[0-9A-Za-z._-]+$' ||
  die 'could not read a filesystem-safe package version'
command -v git >/dev/null 2>&1 || die 'git is required to fingerprint the source tree'
fingerprint_source
SOURCE_HASH="$SOURCE_HASH_RESULT"
BUILD_ID="${PACKAGE_VERSION}-dev.${SOURCE_HASH}"

BUNDLE_DIR=""
for candidate in "$DEV_DATA_ROOT/$BUILD_ID" "$DEV_DATA_ROOT/$BUILD_ID".repair.*; do
  [ -e "$candidate" ] || continue
  if valid_bundle "$candidate" "$BUILD_ID"; then BUNDLE_DIR="$candidate"; break; fi
done

if [ -z "$BUNDLE_DIR" ]; then
  CARGO_BIN="${CARGO:-$(command -v cargo 2>/dev/null || true)}"
  RUSTC_BIN="${RUSTC:-$(command -v rustc 2>/dev/null || true)}"
  [ -n "$CARGO_BIN" ] || CARGO_BIN="${HOME}/.cargo/bin/cargo"
  [ -n "$RUSTC_BIN" ] || RUSTC_BIN="${HOME}/.cargo/bin/rustc"
  [ -x "$CARGO_BIN" ] || die "cargo is not executable: $CARGO_BIN"
  [ -x "$RUSTC_BIN" ] || die "rustc is not executable: $RUSTC_BIN"
  HOST_TARGET="$("$RUSTC_BIN" -vV | sed -n 's/^host: //p')"
  [ -n "$HOST_TARGET" ] || die 'could not determine the Rust host target'
  BLACKPEPPER_BUILD_ID="$BUILD_ID" "$CARGO_BIN" build \
    --manifest-path "$ROOT/Cargo.toml" --target-dir "$DEV_TARGET_DIR" \
    --target "$HOST_TARGET" -p blackpepper --bin bp --bin bp-host
  fingerprint_source
  [ "$SOURCE_HASH_RESULT" = "$SOURCE_HASH" ] ||
    die 'source tree changed during the build; rerun setup so the build identity stays exact'
  SOURCE_DIR="$DEV_TARGET_DIR/$HOST_TARGET/debug"
  STAGE_DIR="$(mktemp -d "$DEV_DATA_ROOT/.staging.XXXXXX")"
  install -m 0755 "$SOURCE_DIR/bp" "$STAGE_DIR/bp-dev"
  install -m 0755 "$SOURCE_DIR/bp-host" "$STAGE_DIR/bp-host"
  if command -v strip >/dev/null 2>&1; then
    if [ "$(uname -s)" = Darwin ]; then STRIP_ARGS=(-S); else STRIP_ARGS=(--strip-debug); fi
    if ! strip "${STRIP_ARGS[@]}" "$STAGE_DIR/bp-dev" "$STAGE_DIR/bp-host"; then
      log 'Warning: stripping staged binaries failed; installing unstripped copies.' >&2
      install -m 0755 "$SOURCE_DIR/bp" "$STAGE_DIR/bp-dev"
      install -m 0755 "$SOURCE_DIR/bp-host" "$STAGE_DIR/bp-host"
    fi
  else
    log 'Warning: strip is unavailable; the retained development bundle will be large.' >&2
  fi
  [ "$("$STAGE_DIR/bp-dev" --version)" = "blackpepper $BUILD_ID" ] || die 'staged client failed its version check'
  [ "$("$STAGE_DIR/bp-host" --version)" = "bp-host $BUILD_ID" ] || die 'staged helper failed its version check'
  BUNDLE_DIR="$DEV_DATA_ROOT/$BUILD_ID"
  if ! mkdir -m 0700 "$BUNDLE_DIR" 2>/dev/null; then
    BUNDLE_DIR="$(mktemp -d "$DEV_DATA_ROOT/$BUILD_ID.repair.XXXXXX")"
  fi
  install -m 0755 "$STAGE_DIR/bp-dev" "$BUNDLE_DIR/bp-dev"
  install -m 0755 "$STAGE_DIR/bp-host" "$BUNDLE_DIR/bp-host"
  printf '%s\n%s\n' "$MARKER_VERSION" "$BUILD_ID" > "$BUNDLE_DIR/$MARKER_NAME"
  chmod 0600 "$BUNDLE_DIR/$MARKER_NAME"
  rm -rf "$STAGE_DIR"
  STAGE_DIR=""
fi
valid_bundle "$BUNDLE_DIR" "$BUILD_ID" || die "development bundle is invalid: $BUNDLE_DIR"
fingerprint_source
[ "$SOURCE_HASH_RESULT" = "$SOURCE_HASH" ] ||
  die 'source tree changed during installation; rerun setup so the build identity stays exact'

if [ -L "$LINK_ROOT/current" ]; then
  OLD_CURRENT_PRESENT=1
  OLD_CURRENT_TARGET="$(readlink "$LINK_ROOT/current")"
fi
LINK_TEMP="$LINK_ROOT/.current.$$"
ln -s "$BUNDLE_DIR" "$LINK_TEMP"
replace_path "$WRAPPER_TEMP" "$DEV_INSTALL_DIR/bp-dev"
WRAPPER_TEMP=""
WRAPPER_PUBLISHED=1
if [ "${BLACKPEPPER_TEST_FAIL_BEFORE_ACTIVATION:-0}" = 1 ]; then
  die 'injected failure before development activation'
fi
replace_path "$LINK_TEMP" "$LINK_ROOT/current"
LINK_TEMP=""
CURRENT_SWITCHED=1
[ "$("$DEV_INSTALL_DIR/bp-dev" --version)" = "blackpepper $BUILD_ID" ] ||
  die 'installed development client failed its version check'
INSTALL_SUCCEEDED=1

RETAIN_LIMIT="${BLACKPEPPER_DEV_RETAIN_BUILDS:-5}"
printf '%s\n' "$RETAIN_LIMIT" | grep -Eq '^[1-9][0-9]*$' || die 'BLACKPEPPER_DEV_RETAIN_BUILDS must be a positive integer'
BUNDLE_COUNT=0
for candidate in "$DEV_DATA_ROOT"/*; do
  [ -e "$candidate" ] && BUNDLE_COUNT=$((BUNDLE_COUNT + 1))
done
BUNDLE_SIZE="$(du -sh "$DEV_DATA_ROOT" 2>/dev/null | awk '{print $1}')"
log "Installed development build: $DEV_INSTALL_DIR/bp-dev"
log "Build identity: $BUILD_ID"
log 'Production bp and bp-host were not changed.'
log "Retained development storage: ${BUNDLE_SIZE:-unknown-size} across $BUNDLE_COUNT entries."
log 'No bundles were auto-deleted; provider hooks may use exact paths.'
if [ "$BUNDLE_COUNT" -gt "$RETAIN_LIMIT" ]; then
  log "Warning: retained bundle count exceeds $RETAIN_LIMIT. Review with: scripts/setup.sh --list-builds" >&2
  log 'Remove one exact inactive bundle with: scripts/setup.sh --prune BUNDLE_NAME' >&2
fi
