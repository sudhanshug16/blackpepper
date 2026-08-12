#!/usr/bin/env bash
set -euo pipefail

REPO="sudhanshug16/blackpepper"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
RELEASE_BASE_URL="${BLACKPEPPER_RELEASE_BASE_URL:-https://github.com/${REPO}/releases/latest/download}"

log() {
  printf '%s\n' "$*"
}

die() {
  log "Error: $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1
}

fetch() {
  local url="$1"
  local out="$2"

  if need_cmd curl; then
    curl -fsSL "$url" -o "$out"
    return
  fi
  if need_cmd wget; then
    wget -qO "$out" "$url"
    return
  fi
  die "curl or wget is required to download the release"
}

sha256_file() {
  local path="$1"
  if need_cmd sha256sum; then
    sha256sum "$path" | awk '{print $1}'
    return
  fi
  if need_cmd shasum; then
    shasum -a 256 "$path" | awk '{print $1}'
    return
  fi
  die "sha256sum or shasum is required to verify the release"
}

manifest_digest() {
  local manifest="$1"
  local filename="$2"
  awk -v filename="$filename" '
    $2 == filename || $2 == "*" filename {
      count += 1
      digest = $1
    }
    END {
      if (count != 1) exit 1
      print digest
    }
  ' "$manifest"
}

verify_digest() {
  local path="$1"
  local expected="$2"
  local actual

  case "$expected" in
    ''|*[!0-9a-fA-F]*) die "invalid SHA-256 for $(basename "$path")" ;;
  esac
  [ "${#expected}" -eq 64 ] || die "invalid SHA-256 for $(basename "$path")"
  actual="$(sha256_file "$path")"
  [ "$(printf '%s' "$actual" | tr '[:upper:]' '[:lower:]')" = \
    "$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')" ] || \
    die "checksum mismatch for $(basename "$path"); nothing was installed"
}

replace_path() {
  local source="$1"
  local destination="$2"
  case "$OS" in
    darwin) mv -fh "$source" "$destination" ;;
    linux) mv -Tf -- "$source" "$destination" ;;
    *) die "unsupported OS while publishing symlink: $OS" ;;
  esac
}

validate_archive_entries() {
  local archive="$1"
  local actual="$2"
  local expected="$3"

  tar -tzf "$archive" > "$actual"
  printf '%s\n' \
    LICENSE \
    LICENSE-HERDR-APACHE-2.0 \
    THIRD_PARTY_NOTICES.md \
    VERSION \
    SHA256SUMS \
    bp \
    bp-host \
    sidecars/aarch64-unknown-linux-musl/bp-host \
    sidecars/x86_64-unknown-linux-musl/bp-host \
    | LC_ALL=C sort > "$expected"
  LC_ALL=C sort "$actual" -o "$actual"
  cmp -s "$actual" "$expected" || \
    die "release archive has an unexpected or incomplete file layout"
}

verify_payload() {
  local root="$1"
  local relative expected
  local files="LICENSE LICENSE-HERDR-APACHE-2.0 THIRD_PARTY_NOTICES.md VERSION bp bp-host sidecars/aarch64-unknown-linux-musl/bp-host sidecars/x86_64-unknown-linux-musl/bp-host"

  if [ ! -f "$root/SHA256SUMS" ] || [ -L "$root/SHA256SUMS" ]; then
    die "release checksum manifest is not a regular file"
  fi
  for relative in $files; do
    if [ ! -f "$root/$relative" ] || [ -L "$root/$relative" ]; then
      die "release file $relative is not a regular file"
    fi
    expected="$(manifest_digest "$root/SHA256SUMS" "$relative")" || \
      die "release checksum manifest does not name $relative exactly once"
    verify_digest "$root/$relative" "$expected"
  done
}

payload_matches() {
  local installed="$1"
  local source="$2"
  local relative
  local files="LICENSE LICENSE-HERDR-APACHE-2.0 THIRD_PARTY_NOTICES.md VERSION SHA256SUMS bp bp-host sidecars/aarch64-unknown-linux-musl/bp-host sidecars/x86_64-unknown-linux-musl/bp-host"

  for relative in $files; do
    [ -f "$installed/$relative" ] && [ ! -L "$installed/$relative" ] || return 1
    [ "$(sha256_file "$installed/$relative")" = "$(sha256_file "$source/$relative")" ] || \
      return 1
  done
}

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin) OS="darwin" ;;
  Linux) OS="linux" ;;
  *) die "unsupported OS: $OS" ;;
esac

case "$ARCH" in
  x86_64|amd64) ARCH="x86_64" ;;
  arm64|aarch64) ARCH="arm64" ;;
  *) die "unsupported architecture: $ARCH" ;;
esac

case "$INSTALL_DIR" in
  /*) ;;
  *) die "INSTALL_DIR must be an absolute path" ;;
esac

ASSET="bp-${OS}-${ARCH}.tar.gz"
URL="${RELEASE_BASE_URL}/${ASSET}"
CHECKSUM_URL="${RELEASE_BASE_URL}/SHA256SUMS"

TMP_DIR="$(mktemp -d)"
STAGE_DIR=""
CURRENT_TEMP=""
HOST_TEMP=""
WRAPPER_TEMP=""
BP_BACKUP=""
HOST_BACKUP=""
BP_EXISTED=0
HOST_EXISTED=0
BP_PUBLISHED=0
HOST_PUBLISHED=0
CURRENT_SWITCHED=0
OLD_CURRENT_PRESENT=0
OLD_CURRENT_TARGET=""
INSTALL_COMMITTED=0

cleanup() {
  rm -rf "$TMP_DIR"
  if [ -n "$STAGE_DIR" ]; then
    rm -rf "$STAGE_DIR"
  fi
  if [ -n "$CURRENT_TEMP" ]; then
    rm -f "$CURRENT_TEMP"
  fi
  if [ -n "$HOST_TEMP" ]; then
    rm -f "$HOST_TEMP"
  fi
  if [ -n "$WRAPPER_TEMP" ]; then
    rm -f "$WRAPPER_TEMP"
  fi
}

restore_entrypoint() {
  local destination="$1"
  local backup="$2"
  local existed="$3"
  local temporary="$4"

  rm -f "$temporary" || return 1
  if [ "$existed" -eq 0 ]; then
    rm -f "$destination" || return 1
    return 0
  fi

  if [ -L "$backup" ]; then
    ln -s "$(readlink "$backup")" "$temporary" || return 1
  elif [ -f "$backup" ]; then
    cp -p "$backup" "$temporary" || return 1
  else
    return 1
  fi
  replace_path "$temporary" "$destination" || return 1
}

rollback_install() {
  local rollback_failed=0
  local rollback_temp

  if [ "$CURRENT_SWITCHED" -eq 1 ]; then
    rollback_temp="$PACKAGE_ROOT/.current-rollback.$$"
    rm -f "$rollback_temp"
    if [ "$OLD_CURRENT_PRESENT" -eq 1 ]; then
      if ! ln -s "$OLD_CURRENT_TARGET" "$rollback_temp" || \
        ! replace_path "$rollback_temp" "$PACKAGE_ROOT/current"; then
        log "Error: failed to restore ${PACKAGE_ROOT}/current" >&2
        rollback_failed=1
      fi
    elif ! rm -f "$PACKAGE_ROOT/current"; then
      log "Error: failed to remove newly installed ${PACKAGE_ROOT}/current" >&2
      rollback_failed=1
    fi
    rm -f "$rollback_temp"
  fi

  if [ "$HOST_PUBLISHED" -eq 1 ]; then
    if ! restore_entrypoint \
      "$INSTALL_DIR/bp-host" \
      "$HOST_BACKUP" \
      "$HOST_EXISTED" \
      "$INSTALL_DIR/.bp-host-rollback.$$"; then
      log "Error: failed to restore ${INSTALL_DIR}/bp-host" >&2
      rollback_failed=1
    fi
  fi
  if [ "$BP_PUBLISHED" -eq 1 ]; then
    if ! restore_entrypoint \
      "$INSTALL_DIR/bp" \
      "$BP_BACKUP" \
      "$BP_EXISTED" \
      "$INSTALL_DIR/.bp-rollback.$$"; then
      log "Error: failed to restore ${INSTALL_DIR}/bp" >&2
      rollback_failed=1
    fi
  fi

  [ "$rollback_failed" -eq 0 ]
}

finish() {
  local status=$?

  trap - EXIT HUP INT TERM
  if [ "$status" -ne 0 ] && [ "$INSTALL_COMMITTED" -eq 0 ]; then
    if ! rollback_install; then
      log "Error: installer rollback was incomplete; inspect ${INSTALL_DIR}" >&2
      status=1
    fi
  fi
  cleanup
  exit "$status"
}

trap finish EXIT
trap 'exit 1' HUP INT TERM

log "Downloading ${ASSET}..."
fetch "$CHECKSUM_URL" "$TMP_DIR/RELEASE_SHA256SUMS"
fetch "$URL" "$TMP_DIR/${ASSET}"

EXPECTED_ARCHIVE_DIGEST="$(manifest_digest "$TMP_DIR/RELEASE_SHA256SUMS" "$ASSET")" || \
  die "release checksum manifest does not name ${ASSET} exactly once"
verify_digest "$TMP_DIR/${ASSET}" "$EXPECTED_ARCHIVE_DIGEST"
ACTUAL_ARCHIVE_DIGEST="$(sha256_file "$TMP_DIR/${ASSET}")"

validate_archive_entries \
  "$TMP_DIR/${ASSET}" \
  "$TMP_DIR/archive-entries" \
  "$TMP_DIR/expected-entries"
mkdir "$TMP_DIR/payload"
tar -xzf "$TMP_DIR/${ASSET}" -C "$TMP_DIR/payload"
verify_payload "$TMP_DIR/payload"

VERSION="$(tr -d '\r\n' < "$TMP_DIR/payload/VERSION")"
printf '%s\n' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$' || \
  die "release contains an invalid version"
[ "$("$TMP_DIR/payload/bp" --version)" = "blackpepper ${VERSION}" ] || \
  die "bp version does not match the release"
[ "$("$TMP_DIR/payload/bp-host" --version)" = "bp-host ${VERSION}" ] || \
  die "bp-host version does not match the release"

PACKAGE_ROOT="$INSTALL_DIR/.blackpepper"
RELEASES_DIR="$PACKAGE_ROOT/releases"
if [ -L "$PACKAGE_ROOT" ] || [ -L "$RELEASES_DIR" ]; then
  die "refusing to install through a symbolic package directory"
fi
mkdir -p "$INSTALL_DIR"
install -d -m 755 "$PACKAGE_ROOT" "$RELEASES_DIR"

DIGEST_PREFIX="$(printf '%s' "$ACTUAL_ARCHIVE_DIGEST" | cut -c1-12)"
PAYLOAD_NAME="${VERSION}-${OS}-${ARCH}-${DIGEST_PREFIX}"
PAYLOAD_DIR="$RELEASES_DIR/$PAYLOAD_NAME"
if [ -e "$PAYLOAD_DIR" ] && \
  { [ ! -d "$PAYLOAD_DIR" ] || [ -L "$PAYLOAD_DIR" ] || \
    ! payload_matches "$PAYLOAD_DIR" "$TMP_DIR/payload"; }; then
  PAYLOAD_NAME="${PAYLOAD_NAME}-repair-$$"
  PAYLOAD_DIR="$RELEASES_DIR/$PAYLOAD_NAME"
  [ ! -e "$PAYLOAD_DIR" ] || die "verified repair target already exists: $PAYLOAD_DIR"
fi

if [ ! -e "$PAYLOAD_DIR" ]; then
  STAGE_DIR="$(mktemp -d "$RELEASES_DIR/.staging.XXXXXX")"
  install -d -m 755 \
    "$STAGE_DIR/sidecars/aarch64-unknown-linux-musl" \
    "$STAGE_DIR/sidecars/x86_64-unknown-linux-musl"
  install -m 755 "$TMP_DIR/payload/bp" "$STAGE_DIR/bp"
  install -m 755 "$TMP_DIR/payload/bp-host" "$STAGE_DIR/bp-host"
  install -m 755 \
    "$TMP_DIR/payload/sidecars/aarch64-unknown-linux-musl/bp-host" \
    "$STAGE_DIR/sidecars/aarch64-unknown-linux-musl/bp-host"
  install -m 755 \
    "$TMP_DIR/payload/sidecars/x86_64-unknown-linux-musl/bp-host" \
    "$STAGE_DIR/sidecars/x86_64-unknown-linux-musl/bp-host"
  install -m 644 "$TMP_DIR/payload/LICENSE" "$STAGE_DIR/LICENSE"
  install -m 644 \
    "$TMP_DIR/payload/LICENSE-HERDR-APACHE-2.0" \
    "$STAGE_DIR/LICENSE-HERDR-APACHE-2.0"
  install -m 644 \
    "$TMP_DIR/payload/THIRD_PARTY_NOTICES.md" \
    "$STAGE_DIR/THIRD_PARTY_NOTICES.md"
  install -m 644 "$TMP_DIR/payload/VERSION" "$STAGE_DIR/VERSION"
  install -m 644 "$TMP_DIR/payload/SHA256SUMS" "$STAGE_DIR/SHA256SUMS"
  mv "$STAGE_DIR" "$PAYLOAD_DIR"
  STAGE_DIR=""
fi

if [ -d "$INSTALL_DIR/bp" ] && [ ! -L "$INSTALL_DIR/bp" ]; then
  die "refusing to replace directory ${INSTALL_DIR}/bp"
fi
if [ -d "$INSTALL_DIR/bp-host" ] && [ ! -L "$INSTALL_DIR/bp-host" ]; then
  die "refusing to replace directory ${INSTALL_DIR}/bp-host"
fi
if [ -e "$PACKAGE_ROOT/current" ] && [ ! -L "$PACKAGE_ROOT/current" ]; then
  die "refusing to replace non-symbolic ${PACKAGE_ROOT}/current"
fi

WRAPPER_TEMP="$INSTALL_DIR/.bp-install.$$"
# These variables intentionally expand when the installed launcher runs.
# shellcheck disable=SC2016
printf '%s\n' \
  '#!/bin/sh' \
  'set -eu' \
  'SELF="$0"' \
  'case "$SELF" in */*) ;; *) SELF=$(command -v "$SELF") ;; esac' \
  'SELF_DIR=$(CDPATH= cd -P "$(dirname "$SELF")" && pwd)' \
  '# Keep automatic updates in the directory selected during installation.' \
  'INSTALL_DIR="$SELF_DIR"' \
  'export INSTALL_DIR' \
  'TARGET="$SELF_DIR/.blackpepper/current/bp"' \
  'if [ ! -x "$TARGET" ]; then' \
  '  printf '\''Blackpepper installation is incomplete: %s is missing.\n'\'' "$TARGET" >&2' \
  '  exit 126' \
  'fi' \
  'exec "$TARGET" "$@"' \
  > "$WRAPPER_TEMP"
chmod 755 "$WRAPPER_TEMP"
sh -n "$WRAPPER_TEMP" || die "generated client entrypoint failed validation"

HOST_TEMP="$INSTALL_DIR/.bp-host-install.$$"
ln -s ".blackpepper/current/bp-host" "$HOST_TEMP"
[ "$(readlink "$HOST_TEMP")" = ".blackpepper/current/bp-host" ] || \
  die "generated helper entrypoint failed validation"

CURRENT_TEMP="$PACKAGE_ROOT/.current.$$"
ln -s "releases/$PAYLOAD_NAME" "$CURRENT_TEMP"
[ "$(readlink "$CURRENT_TEMP")" = "releases/$PAYLOAD_NAME" ] || \
  die "generated release pointer failed validation"
[ "$("$CURRENT_TEMP/bp" --version)" = "blackpepper ${VERSION}" ] || \
  die "staged client failed its version check"
[ "$("$CURRENT_TEMP/bp-host" --version)" = "bp-host ${VERSION}" ] || \
  die "staged helper failed its version check"

# Snapshot both old entrypoints before publishing either one. They are
# published while current still names the old payload, so activation remains
# one atomic symlink replacement after every other path has been validated.
BP_BACKUP="$TMP_DIR/bp.backup"
if [ -L "$INSTALL_DIR/bp" ]; then
  ln -s "$(readlink "$INSTALL_DIR/bp")" "$BP_BACKUP"
  BP_EXISTED=1
elif [ -f "$INSTALL_DIR/bp" ]; then
  cp -p "$INSTALL_DIR/bp" "$BP_BACKUP"
  BP_EXISTED=1
elif [ -e "$INSTALL_DIR/bp" ]; then
  die "refusing to replace non-file ${INSTALL_DIR}/bp"
fi

HOST_BACKUP="$TMP_DIR/bp-host.backup"
if [ -L "$INSTALL_DIR/bp-host" ]; then
  ln -s "$(readlink "$INSTALL_DIR/bp-host")" "$HOST_BACKUP"
  HOST_EXISTED=1
elif [ -f "$INSTALL_DIR/bp-host" ]; then
  cp -p "$INSTALL_DIR/bp-host" "$HOST_BACKUP"
  HOST_EXISTED=1
elif [ -e "$INSTALL_DIR/bp-host" ]; then
  die "refusing to replace non-file ${INSTALL_DIR}/bp-host"
fi

if [ -L "$PACKAGE_ROOT/current" ]; then
  OLD_CURRENT_TARGET="$(readlink "$PACKAGE_ROOT/current")"
  OLD_CURRENT_PRESENT=1
fi

BP_PUBLISHED=1
replace_path "$WRAPPER_TEMP" "$INSTALL_DIR/bp"
WRAPPER_TEMP=""

HOST_PUBLISHED=1
replace_path "$HOST_TEMP" "$INSTALL_DIR/bp-host"
HOST_TEMP=""

if [ "${BLACKPEPPER_TEST_FAIL_BEFORE_ACTIVATION:-}" = "1" ]; then
  die "injected failure before activation"
fi

# The current pointer is the activation point and final filesystem mutation.
CURRENT_SWITCHED=1
replace_path "$CURRENT_TEMP" "$PACKAGE_ROOT/current"
CURRENT_TEMP=""

[ "$("$INSTALL_DIR/bp" --version)" = "blackpepper ${VERSION}" ] || \
  die "installed client failed its version check"
[ "$("$INSTALL_DIR/bp-host" --version)" = "bp-host ${VERSION}" ] || \
  die "installed helper failed its version check"
INSTALL_COMMITTED=1

log "Installed Blackpepper ${VERSION} to ${PAYLOAD_DIR}"
log "Client entrypoint: ${INSTALL_DIR}/bp"
if ! command -v bp >/dev/null 2>&1; then
  log "Note: ${INSTALL_DIR} is not in PATH"
  log "Add this to your shell profile:"
  log "  export PATH=\"${INSTALL_DIR}:\$PATH\""
fi
