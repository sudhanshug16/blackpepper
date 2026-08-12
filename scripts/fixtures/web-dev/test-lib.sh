#!/usr/bin/env bash

# Shared fixture builders for test-web-dev.sh. The caller defines fail().

file_mode() {
  stat -c '%a' "$1" 2>/dev/null || stat -f '%Lp' "$1"
}

make_bp_dev() {
  local install_root="$1" bundle_root="$1/.blackpepper-dev/test-bundle"
  install -d -m 0755 "$bundle_root"
  cat > "$bundle_root/bp-dev" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod 0755 "$bundle_root/bp-dev"
  ln -s test-bundle "$install_root/.blackpepper-dev/current"
}

make_exact_ttyd() {
  local bin_root="$1"
  install -d -m 0755 "$bin_root"
  cat > "$bin_root/ttyd" <<'EOF'
#!/bin/sh
if [ "${1:-}" = --version ]; then
  printf '%s\n' 'ttyd version 1.7.7-40e79c7'
  exit 0
fi
: "${FAKE_TTYD_ARGS:?}"
printf '%s\n' "$@" > "$FAKE_TTYD_ARGS"
EOF
  chmod 0755 "$bin_root/ttyd"
}

assert_arg() {
  local actual="$1" expected="$2" position="$3"
  [ "$actual" = "$expected" ] ||
    fail "argument $position: expected '$expected', got '$actual'"
}
