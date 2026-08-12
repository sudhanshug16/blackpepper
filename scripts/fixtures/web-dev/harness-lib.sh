#!/usr/bin/env bash

# ttyd acquisition and custom browser-index construction for web-dev.sh.
# The caller defines ROOT, ttyd version/cache constants, CLIPBOARD_BRIDGE, and
# die(). This file is sourced so the final exec remains the ttyd process.

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
    return
  fi
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
    return
  fi
  die 'sha256sum or shasum is required to verify ttyd'
}

absolute_command() {
  local candidate="$1" directory name
  case "$candidate" in
    /*) printf '%s\n' "$candidate"; return ;;
    */*) directory="$(CDPATH='' cd -- "$(dirname "$candidate")" && pwd)" ;;
    *)
      candidate="$(command -v "$candidate" 2>/dev/null || true)"
      [ -n "$candidate" ] || return 1
      case "$candidate" in
        /*) printf '%s\n' "$candidate"; return ;;
      esac
      directory="$(CDPATH='' cd -- "$(dirname "$candidate")" && pwd)"
      ;;
  esac
  name="$(basename "$candidate")"
  printf '%s/%s\n' "$directory" "$name"
}

ttyd_is_compatible_system() {
  local output
  [ -f "$1" ] && [ -x "$1" ] || return 1
  output="$("$1" --version 2>/dev/null || true)"
  case "$output" in
    "$TTYD_SYSTEM_VERSION_OUTPUT"|"$TTYD_SYSTEM_VERSION_OUTPUT"-*) return 0 ;;
    *) return 1 ;;
  esac
}

ttyd_has_managed_identity() {
  [ -f "$1" ] && [ -x "$1" ] &&
    [ "$("$1" --version 2>/dev/null || true)" = "$TTYD_MANAGED_VERSION_OUTPUT" ]
}

download() {
  local url="$1" destination="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$destination"
    return
  fi
  if command -v wget >/dev/null 2>&1; then
    wget -qO "$destination" "$url"
    return
  fi
  die 'curl or wget is required to download ttyd'
}

ensure_ttyd() {
  local system_ttyd="" os architecture asset expected managed actual temporary
  system_ttyd="$(absolute_command ttyd 2>/dev/null || true)"
  if [ -n "$system_ttyd" ] && ttyd_is_compatible_system "$system_ttyd"; then
    printf '%s\n' "$system_ttyd"
    return
  fi

  os="$(uname -s)"
  if [ "$os" = Darwin ]; then
    die "ttyd ${TTYD_VERSION} is required on macOS; install it with: brew install ttyd"
  fi
  [ "$os" = Linux ] || die "unsupported development host for managed ttyd: $os"

  architecture="$(uname -m)"
  case "$architecture" in
    x86_64|amd64)
      asset='ttyd.x86_64'
      expected='8a217c968aba172e0dbf3f34447218dc015bc4d5e59bf51db2f2cd12b7be4f55'
      ;;
    aarch64|arm64)
      asset='ttyd.aarch64'
      expected='b38acadd89d1d396a0f5649aa52c539edbad07f4bc7348b27b4f4b7219dd4165'
      ;;
    *) die "no checksum-pinned ttyd ${TTYD_VERSION} asset for Linux architecture: $architecture" ;;
  esac

  [ ! -L "$TTYD_CACHE_ROOT" ] ||
    die "refusing symbolic ttyd cache directory: $TTYD_CACHE_ROOT"
  install -d -m 0700 "$TTYD_CACHE_ROOT"
  managed="$TTYD_CACHE_ROOT/$asset"
  if [ -e "$managed" ] || [ -L "$managed" ]; then
    if [ ! -f "$managed" ] || [ -L "$managed" ]; then
      die "refusing invalid managed ttyd path: $managed"
    fi
    actual="$(sha256_file "$managed")"
    [ "$actual" = "$expected" ] ||
      die "managed ttyd checksum mismatch at $managed; remove that exact file and retry"
    chmod 0700 "$managed"
    ttyd_has_managed_identity "$managed" ||
      die "managed ttyd failed its exact ${TTYD_VERSION} version check: $managed"
    printf '%s\n' "$managed"
    return
  fi

  temporary="$(mktemp "$TTYD_CACHE_ROOT/.${asset}.download.XXXXXX")"
  trap 'rm -f -- "$temporary"' EXIT
  trap 'exit 129' HUP
  trap 'exit 130' INT
  trap 'exit 143' TERM
  download "$TTYD_RELEASE_BASE_URL/$asset" "$temporary"
  actual="$(sha256_file "$temporary")"
  [ "$actual" = "$expected" ] ||
    die "downloaded ttyd checksum mismatch for $asset (expected $expected, got $actual)"
  chmod 0700 "$temporary"
  ttyd_has_managed_identity "$temporary" ||
    die "downloaded ttyd failed its exact ${TTYD_VERSION} version check"

  # A hard link publishes without replacing a concurrently created cache file.
  if ln "$temporary" "$managed" 2>/dev/null; then
    rm -f -- "$temporary"
  else
    if [ ! -f "$managed" ] || [ -L "$managed" ] ||
      [ "$(sha256_file "$managed")" != "$expected" ] ||
      ! ttyd_has_managed_identity "$managed"; then
      die "another process created an invalid managed ttyd at $managed"
    fi
    rm -f -- "$temporary"
  fi
  trap - EXIT HUP INT TERM
  printf '%s\n' "$managed"
}

fetch_local_index() {
  local url="$1" destination="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsS --max-time 5 "$url" -o "$destination"
    return
  fi
  if command -v wget >/dev/null 2>&1; then
    wget -q -T 5 -O "$destination" "$url"
    return
  fi
  die 'curl or wget is required to prepare the ttyd clipboard bridge'
}

INDEX_PROBE_PID=''
INDEX_TEMP_DEFAULT=''
INDEX_TEMP_CANDIDATE=''
INDEX_TEMP_LOG=''

cleanup_index_build() {
  if [ -n "$INDEX_PROBE_PID" ]; then
    kill "$INDEX_PROBE_PID" 2>/dev/null || true
    wait "$INDEX_PROBE_PID" 2>/dev/null || true
    INDEX_PROBE_PID=''
  fi
  [ -z "$INDEX_TEMP_DEFAULT" ] || rm -f -- "$INDEX_TEMP_DEFAULT"
  [ -z "$INDEX_TEMP_CANDIDATE" ] || rm -f -- "$INDEX_TEMP_CANDIDATE"
  [ -z "$INDEX_TEMP_LOG" ] || rm -f -- "$INDEX_TEMP_LOG"
  INDEX_TEMP_DEFAULT=''
  INDEX_TEMP_CANDIDATE=''
  INDEX_TEMP_LOG=''
}

valid_browser_index() {
  [ -f "$1" ] && [ ! -L "$1" ] &&
    grep -Fq 'data-blackpepper-clipboard-bridge="1"' "$1" &&
    grep -Fq '__blackpepperClipboardBridge' "$1"
}

ensure_browser_index() {
  local ttyd="$1" identity index_root index port='' iteration=0 size keep
  if [ ! -f "$CLIPBOARD_BRIDGE" ] || [ -L "$CLIPBOARD_BRIDGE" ]; then
    die "clipboard bridge fixture is missing or unsafe: $CLIPBOARD_BRIDGE"
  fi
  identity="$(sha256_file "$ttyd")-$(sha256_file "$CLIPBOARD_BRIDGE")"
  index_root="$TTYD_CACHE_ROOT/browser-index/$identity"
  [ ! -L "$index_root" ] || die "refusing symbolic ttyd index cache directory: $index_root"
  install -d -m 0700 "$index_root"
  index="$index_root/index.html"
  if [ -e "$index" ] || [ -L "$index" ]; then
    valid_browser_index "$index" ||
      die "managed ttyd browser index is invalid; remove this exact file and retry: $index"
    printf '%s\n' "$index"
    return
  fi

  INDEX_TEMP_DEFAULT="$(mktemp "$index_root/.default.XXXXXX")"
  INDEX_TEMP_CANDIDATE="$(mktemp "$index_root/.candidate.XXXXXX")"
  INDEX_TEMP_LOG="$(mktemp "$index_root/.probe.XXXXXX")"
  trap 'cleanup_index_build' EXIT
  trap 'cleanup_index_build; exit 129' HUP
  trap 'cleanup_index_build; exit 130' INT
  trap 'cleanup_index_build; exit 143' TERM

  if [ -n "$TTYD_INDEX_SOURCE" ]; then
    case "$TTYD_INDEX_SOURCE" in
      /*) ;;
      *) die "BLACKPEPPER_WEB_TTYD_INDEX_SOURCE must be absolute: $TTYD_INDEX_SOURCE" ;;
    esac
    if [ ! -f "$TTYD_INDEX_SOURCE" ] || [ -L "$TTYD_INDEX_SOURCE" ]; then
      die "BLACKPEPPER_WEB_TTYD_INDEX_SOURCE is not a regular file: $TTYD_INDEX_SOURCE"
    fi
    cp "$TTYD_INDEX_SOURCE" "$INDEX_TEMP_DEFAULT"
  else
    "$ttyd" -i 127.0.0.1 -p 0 -d 7 /bin/true > "$INDEX_TEMP_LOG" 2>&1 &
    INDEX_PROBE_PID=$!
    while [ "$iteration" -lt 250 ]; do
      port="$(sed -n 's/.*Listening on port: \([0-9][0-9]*\).*/\1/p' \
        "$INDEX_TEMP_LOG" | tail -n 1)"
      [ -z "$port" ] || break
      kill -0 "$INDEX_PROBE_PID" 2>/dev/null ||
        die 'temporary ttyd index probe exited before listening'
      sleep 0.02
      iteration=$((iteration + 1))
    done
    case "$port" in
      ''|*[!0-9]*) die 'temporary ttyd index probe did not publish a loopback port' ;;
    esac
    fetch_local_index "http://127.0.0.1:$port/" "$INDEX_TEMP_DEFAULT"
    kill "$INDEX_PROBE_PID" 2>/dev/null || true
    wait "$INDEX_PROBE_PID" 2>/dev/null || true
    INDEX_PROBE_PID=''
  fi

  [ "$(tail -c 14 "$INDEX_TEMP_DEFAULT")" = '</body></html>' ] ||
    die 'ttyd default page has an unsupported structure; clipboard bridge was not injected'
  size="$(wc -c < "$INDEX_TEMP_DEFAULT" | tr -d ' ')"
  case "$size" in
    ''|*[!0-9]*) die 'could not measure ttyd default page' ;;
  esac
  [ "$size" -gt 14 ] || die 'ttyd default page is unexpectedly empty'
  keep=$((size - 14))
  {
    head -c "$keep" "$INDEX_TEMP_DEFAULT"
    printf '\n'
    sed 's/[[:space:]]*$//' "$CLIPBOARD_BRIDGE"
    printf '%s' '</body></html>'
  } > "$INDEX_TEMP_CANDIDATE"
  chmod 0600 "$INDEX_TEMP_CANDIDATE"
  valid_browser_index "$INDEX_TEMP_CANDIDATE" ||
    die 'generated ttyd browser index is missing its clipboard bridge'

  if ln "$INDEX_TEMP_CANDIDATE" "$index" 2>/dev/null; then
    :
  elif ! valid_browser_index "$index"; then
    die "another process created an invalid ttyd browser index: $index"
  fi
  cleanup_index_build
  trap - EXIT HUP INT TERM
  printf '%s\n' "$index"
}
