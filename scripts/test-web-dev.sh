#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/blackpepper-web-dev-test.XXXXXX")"
INDEX_SOURCE="$TEST_ROOT/default-index.html"
printf '%s' '<!doctype html><html><body><script>window.term={};</script></body></html>' > "$INDEX_SOURCE"

cleanup() {
  rm -rf -- "$TEST_ROOT"
}
trap cleanup EXIT HUP INT TERM

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

# shellcheck source=scripts/fixtures/web-dev/test-lib.sh
source "$ROOT/scripts/fixtures/web-dev/test-lib.sh"

test_secure_exact_argv() {
  local case_root="$TEST_ROOT/secure" fake_bin="$TEST_ROOT/secure/bin"
  local install_root="$TEST_ROOT/secure/install" args_file="$TEST_ROOT/secure/args"
  local output="$TEST_ROOT/secure/output" argument auth_secret path_secret
  local -a args
  make_bp_dev "$install_root"
  make_exact_ttyd "$fake_bin"

  PATH="$fake_bin:$PATH" \
    DEV_INSTALL_DIR="$install_root" \
    BLACKPEPPER_WEB_TTYD_CACHE_DIR="$case_root/cache" \
    BLACKPEPPER_WEB_TTYD_INDEX_SOURCE="$INDEX_SOURCE" \
    FAKE_TTYD_ARGS="$args_file" \
    "$ROOT/scripts/web-dev.sh" --skip-build --port 19077 > "$output"

  args=()
  while IFS= read -r argument; do
    args[${#args[@]}]="$argument"
  done < "$args_file"
  [ "${#args[@]}" -eq 38 ] || fail "expected 38 ttyd arguments, got ${#args[@]}"
  assert_arg "${args[0]}" '-i' 0
  assert_arg "${args[1]}" '127.0.0.1' 1
  assert_arg "${args[2]}" '-p' 2
  assert_arg "${args[3]}" '19077' 3
  assert_arg "${args[4]}" '-W' 4
  assert_arg "${args[5]}" '-O' 5
  assert_arg "${args[6]}" '-m' 6
  assert_arg "${args[7]}" '1' 7
  assert_arg "${args[8]}" '-o' 8
  assert_arg "${args[9]}" '-c' 9
  auth_secret="${args[10]#bp:}"
  case "$auth_secret" in
    *[!0-9a-f]*|'') fail 'Basic-auth password is not hexadecimal' ;;
  esac
  [ "${#auth_secret}" -eq 32 ] || fail 'Basic-auth password is not 128-bit random hex'
  assert_arg "${args[11]}" '-d' 11
  assert_arg "${args[12]}" '3' 12
  assert_arg "${args[13]}" '-T' 13
  assert_arg "${args[14]}" 'xterm-256color' 14
  assert_arg "${args[15]}" '-b' 15
  path_secret="${args[16]#/bp-dev-}"
  case "$path_secret" in
    *[!0-9a-f]*|'') fail "capability path is not hexadecimal: ${args[16]}" ;;
  esac
  [ "${#path_secret}" -eq 32 ] ||
    fail "capability path is not 128-bit random hex: ${args[16]}"
  [ "$auth_secret" != "$path_secret" ] ||
    fail 'Basic-auth password must be independent from the capability path'
  assert_arg "${args[17]}" '-w' 17
  assert_arg "${args[18]}" "$ROOT" 18
  assert_arg "${args[19]}" '-I' 19
  case "${args[20]}" in
    "$case_root/cache/browser-index/"*/index.html) ;;
    *) fail "browser index is outside its private ttyd cache: ${args[20]}" ;;
  esac
  grep -Fq 'data-blackpepper-clipboard-bridge="1"' "${args[20]}" ||
    fail 'browser index omitted the clipboard bridge'
  [ "$(file_mode "${args[20]}")" = 600 ] ||
    fail 'browser index is not private to the current user'
  grep -Fq 'navigator.clipboard.writeText' "${args[20]}" ||
    fail 'browser index omitted its clipboard write path'
  if grep -Fq 'navigator.clipboard.readText' "${args[20]}"; then
    fail 'browser clipboard bridge must never read clipboard contents'
  fi
  assert_arg "${args[21]}" '-t' 21
  assert_arg "${args[22]}" 'allowProposedApi=true' 22
  assert_arg "${args[23]}" '-t' 23
  assert_arg "${args[24]}" 'rendererType=dom' 24
  assert_arg "${args[25]}" '-t' 25
  assert_arg "${args[26]}" 'screenReaderMode=true' 26
  assert_arg "${args[27]}" '-t' 27
  assert_arg "${args[28]}" 'disableReconnect=true' 28
  assert_arg "${args[29]}" '-t' 29
  assert_arg "${args[30]}" 'disableLeaveAlert=true' 30
  assert_arg "${args[31]}" '-t' 31
  assert_arg "${args[32]}" 'enableZmodem=false' 32
  assert_arg "${args[33]}" '-t' 33
  assert_arg "${args[34]}" 'enableTrzsz=false' 34
  assert_arg "${args[35]}" '-t' 35
  assert_arg "${args[36]}" 'enableSixel=false' 36
  assert_arg "${args[37]}" "$install_root/.blackpepper-dev/test-bundle/bp-dev" 37

  for argument in "${args[@]}"; do
    case "$argument" in
      -a|-6|-B|--url-arg|--ipv6|--browser|sh|bash)
        fail "unsafe or arbitrary-command ttyd argument present: $argument"
        ;;
    esac
  done
  grep -Fqx \
    "Blackpepper browser terminal: http://${args[10]}@127.0.0.1:19077${args[16]}/" \
    "$output" || fail 'printed browser URL does not match the capability path'
}

test_rejects_unpatchable_index() {
  local case_root="$TEST_ROOT/unpatchable" fake_bin="$TEST_ROOT/unpatchable/bin"
  local install_root="$TEST_ROOT/unpatchable/install" output="$TEST_ROOT/unpatchable/output"
  local malformed="$TEST_ROOT/unpatchable/default.html"
  make_bp_dev "$install_root"
  make_exact_ttyd "$fake_bin"
  printf '%s' '<!doctype html><p>missing body suffix</p>' > "$malformed"

  if PATH="$fake_bin:$PATH" \
    DEV_INSTALL_DIR="$install_root" \
    BLACKPEPPER_WEB_TTYD_CACHE_DIR="$case_root/cache" \
    BLACKPEPPER_WEB_TTYD_INDEX_SOURCE="$malformed" \
    FAKE_TTYD_ARGS="$case_root/args" \
    "$ROOT/scripts/web-dev.sh" --skip-build > "$output" 2>&1; then
    fail 'an unpatchable ttyd page was accepted'
  fi
  grep -Fq 'unsupported structure' "$output" ||
    fail 'unpatchable ttyd page failure was not actionable'
  if find "$case_root/cache" -name index.html -type f -print | grep -q .; then
    fail 'an invalid browser index was published'
  fi
}

test_download_checksum_failure() {
  local case_root="$TEST_ROOT/checksum" fake_bin="$TEST_ROOT/checksum/bin"
  local install_root="$TEST_ROOT/checksum/install" output="$TEST_ROOT/checksum/output" remaining
  make_bp_dev "$install_root"
  install -d -m 0755 "$fake_bin"
  cat > "$fake_bin/ttyd" <<'EOF'
#!/bin/sh
printf '%s\n' 'ttyd version 1.7.6'
EOF
  cat > "$fake_bin/curl" <<'EOF'
#!/bin/sh
destination=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) destination="$2"; shift 2 ;;
    *) shift ;;
  esac
done
[ -n "$destination" ] || exit 2
printf '%s\n' 'not a trusted ttyd binary' > "$destination"
EOF
  cat > "$fake_bin/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in
  -s) printf '%s\n' 'Linux' ;;
  -m) printf '%s\n' 'x86_64' ;;
  *) exit 2 ;;
esac
EOF
  chmod 0755 "$fake_bin/ttyd" "$fake_bin/curl" "$fake_bin/uname"

  if PATH="$fake_bin:$PATH" \
    DEV_INSTALL_DIR="$install_root" \
    BLACKPEPPER_WEB_TTYD_CACHE_DIR="$case_root/cache" \
    BLACKPEPPER_WEB_TTYD_RELEASE_BASE_URL='https://invalid.example/ttyd/1.7.7' \
    "$ROOT/scripts/web-dev.sh" --skip-build > "$output" 2>&1; then
    fail 'a corrupt ttyd download was accepted'
  fi
  grep -Fq 'downloaded ttyd checksum mismatch' "$output" ||
    fail 'checksum failure was not actionable'
  remaining="$(find "$case_root/cache" -type f -print)"
  if [ -n "$remaining" ]; then
    fail 'a corrupt ttyd download remained in the managed cache'
  fi
}

test_macos_hint() {
  local case_root="$TEST_ROOT/macos" fake_bin="$TEST_ROOT/macos/bin"
  local install_root="$TEST_ROOT/macos/install" output="$TEST_ROOT/macos/output"
  make_bp_dev "$install_root"
  install -d -m 0755 "$fake_bin"
  cat > "$fake_bin/ttyd" <<'EOF'
#!/bin/sh
printf '%s\n' 'ttyd version 9.9.9'
EOF
  cat > "$fake_bin/uname" <<'EOF'
#!/bin/sh
printf '%s\n' 'Darwin'
EOF
  chmod 0755 "$fake_bin/ttyd" "$fake_bin/uname"

  if PATH="$fake_bin:$PATH" \
    DEV_INSTALL_DIR="$install_root" \
    BLACKPEPPER_WEB_TTYD_CACHE_DIR="$case_root/cache" \
    "$ROOT/scripts/web-dev.sh" --skip-build > "$output" 2>&1; then
    fail 'macOS accepted the wrong installed ttyd version'
  fi
  grep -Fq 'brew install ttyd' "$output" || fail 'macOS failure omitted the brew hint'
}

test_macos_compatible_versions() {
  local case_root="$TEST_ROOT/macos-homebrew" fake_bin="$TEST_ROOT/macos-homebrew/bin"
  local install_root="$TEST_ROOT/macos-homebrew/install"
  local args_file="$TEST_ROOT/macos-homebrew/args" output="$TEST_ROOT/macos-homebrew/output"
  local version_output
  make_bp_dev "$install_root"
  install -d -m 0755 "$fake_bin"
  cat > "$fake_bin/ttyd" <<'EOF'
#!/bin/sh
if [ "${1:-}" = --version ]; then
  printf '%s\n' "${FAKE_TTYD_VERSION_OUTPUT:?}"
  exit 0
fi
: "${FAKE_TTYD_ARGS:?}"
printf '%s\n' "$@" > "$FAKE_TTYD_ARGS"
EOF
  cat > "$fake_bin/uname" <<'EOF'
#!/bin/sh
printf '%s\n' 'Darwin'
EOF
  chmod 0755 "$fake_bin/ttyd" "$fake_bin/uname"

  for version_output in 'ttyd version 1.7.7' 'ttyd version 1.7.7-unknown'; do
    : > "$args_file"
    PATH="$fake_bin:$PATH" \
      DEV_INSTALL_DIR="$install_root" \
      BLACKPEPPER_WEB_TTYD_CACHE_DIR="$case_root/cache" \
      BLACKPEPPER_WEB_TTYD_INDEX_SOURCE="$INDEX_SOURCE" \
      FAKE_TTYD_ARGS="$args_file" \
      FAKE_TTYD_VERSION_OUTPUT="$version_output" \
      "$ROOT/scripts/web-dev.sh" --skip-build > "$output"

    [ -s "$args_file" ] || fail "macOS rejected compatible $version_output"
    tail -n 1 "$args_file" | grep -Fqx \
      "$install_root/.blackpepper-dev/test-bundle/bp-dev" ||
      fail 'macOS-compatible ttyd did not receive the immutable bp-dev path'
  done
}

test_macos_nearby_wrong_version() {
  local case_root="$TEST_ROOT/macos-nearby" fake_bin="$TEST_ROOT/macos-nearby/bin"
  local install_root="$TEST_ROOT/macos-nearby/install" output="$TEST_ROOT/macos-nearby/output"
  make_bp_dev "$install_root"
  install -d -m 0755 "$fake_bin"
  cat > "$fake_bin/ttyd" <<'EOF'
#!/bin/sh
printf '%s\n' 'ttyd version 1.7.70-unknown'
EOF
  cat > "$fake_bin/uname" <<'EOF'
#!/bin/sh
printf '%s\n' 'Darwin'
EOF
  chmod 0755 "$fake_bin/ttyd" "$fake_bin/uname"

  if PATH="$fake_bin:$PATH" \
    DEV_INSTALL_DIR="$install_root" \
    BLACKPEPPER_WEB_TTYD_CACHE_DIR="$case_root/cache" \
    "$ROOT/scripts/web-dev.sh" --skip-build > "$output" 2>&1; then
    fail 'macOS accepted nearby wrong ttyd version 1.7.70'
  fi
  grep -Fq 'ttyd 1.7.7 is required on macOS' "$output" ||
    fail 'nearby wrong macOS ttyd version did not produce the compatibility error'
}

test_secure_exact_argv
test_rejects_unpatchable_index
test_download_checksum_failure
test_macos_hint
test_macos_compatible_versions
test_macos_nearby_wrong_version
printf '%s\n' 'web-dev harness tests passed'
