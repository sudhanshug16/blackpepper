#!/usr/bin/env bash
set -euo pipefail

REPO="sudhanshug16/blackpepper"
BIN_NAME="bp"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

log() {
  printf '%s\n' "$*"
}

die() {
  log "Error: $*"
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

if [ "$OS" = "linux" ] && [ "$ARCH" = "arm64" ]; then
  die "linux arm64 builds are not available yet"
fi

ASSET="bp-${OS}-${ARCH}.tar.gz"
URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

log "Downloading ${ASSET}..."
fetch "$URL" "$TMP_DIR/${ASSET}"

tar -xzf "$TMP_DIR/${ASSET}" -C "$TMP_DIR"
mkdir -p "$INSTALL_DIR"

install -m 755 "$TMP_DIR/${BIN_NAME}" "$INSTALL_DIR/${BIN_NAME}"

log "Installed ${BIN_NAME} to ${INSTALL_DIR}/${BIN_NAME}"
if ! command -v "$BIN_NAME" >/dev/null 2>&1; then
  log "Note: ${INSTALL_DIR} is not in PATH"
  log "Add this to your shell profile:"
  log "  export PATH=\"${INSTALL_DIR}:\$PATH\""
fi
