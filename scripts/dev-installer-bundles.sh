#!/usr/bin/env bash

# Bundle inspection and explicit retention operations for scripts/setup.sh.
# This file is sourced after its path, logging, and marker globals are set.

marker_id() {
  sed -n '2p' "$1/$MARKER_NAME" 2>/dev/null
}

recognized_bundle() {
  local bundle="$1"
  [ -d "$bundle" ] && [ ! -L "$bundle" ] &&
    [ -f "$bundle/$MARKER_NAME" ] && [ ! -L "$bundle/$MARKER_NAME" ] &&
    [ "$(sed -n '1p' "$bundle/$MARKER_NAME")" = "$MARKER_VERSION" ] &&
    [ -n "$(marker_id "$bundle")" ]
}

legacy_bundle() {
  local bundle="$1" build_id="${1##*/}"
  [ -d "$bundle" ] && [ ! -L "$bundle" ] &&
    [ ! -e "$bundle/$MARKER_NAME" ] &&
    [ -x "$bundle/bp-dev" ] && [ ! -L "$bundle/bp-dev" ] &&
    [ -x "$bundle/bp-host" ] && [ ! -L "$bundle/bp-host" ] &&
    [ "$("$bundle/bp-dev" --version 2>/dev/null)" = "blackpepper $build_id" ] &&
    [ "$("$bundle/bp-host" --version 2>/dev/null)" = "bp-host $build_id" ]
}

valid_bundle() {
  local bundle="$1" build_id="$2"
  recognized_bundle "$bundle" && [ "$(marker_id "$bundle")" = "$build_id" ] &&
    [ -x "$bundle/bp-dev" ] && [ ! -L "$bundle/bp-dev" ] &&
    [ -x "$bundle/bp-host" ] && [ ! -L "$bundle/bp-host" ] &&
    [ "$("$bundle/bp-dev" --version 2>/dev/null)" = "blackpepper $build_id" ] &&
    [ "$("$bundle/bp-host" --version 2>/dev/null)" = "bp-host $build_id" ]
}

current_target() {
  local target
  [ -L "$LINK_ROOT/current" ] || return 1
  target="$(readlink "$LINK_ROOT/current")" || return 1
  case "$target" in
    /*) printf '%s\n' "$target" ;;
    *) printf '%s\n' "$LINK_ROOT/$target" ;;
  esac
}

list_bundles() {
  local bundle name size state current="" found=0
  current="$(current_target 2>/dev/null || true)"
  for bundle in "$DEV_DATA_ROOT"/*; do
    [ -e "$bundle" ] || continue
    found=1
    name="${bundle##*/}"
    size="$(du -sh "$bundle" 2>/dev/null | awk '{print $1}')"
    if [ "$bundle" = "$current" ]; then
      state=current
    elif recognized_bundle "$bundle"; then
      state=inactive
    elif legacy_bundle "$bundle"; then
      state=inactive-legacy
    else
      state=unrecognized-retained
    fi
    printf '%s\t%s\t%s\n' "$name" "${size:-unknown-size}" "$state"
  done
  [ "$found" -eq 1 ] || log 'No development bundle entries.'
}

bundle_in_use() {
  local bundle="$1" proc proc_uid exe cmdline my_uid status error_file
  case "$(uname -s)" in
    Linux)
      my_uid="$(id -u)"
      for proc in /proc/[0-9]*; do
        [ -d "$proc" ] || continue
        proc_uid="$(stat -c %u "$proc" 2>/dev/null || true)"
        [ "$proc_uid" = "$my_uid" ] || continue
        exe="$(readlink "$proc/exe" 2>/dev/null || true)"
        case "$exe" in "$bundle"/*) return 0 ;; esac
        if [ -r "$proc/cmdline" ]; then
          cmdline="$(tr '\0' '\n' < "$proc/cmdline" 2>/dev/null || true)"
          case "$cmdline" in *"$bundle/"*) return 0 ;; esac
        elif [ -d "$proc" ]; then
          return 2
        fi
      done
      return 1
      ;;
    Darwin)
      command -v lsof >/dev/null 2>&1 || return 2
      error_file="$(mktemp "${TMPDIR:-/tmp}/blackpepper-lsof.XXXXXX")"
      set +e
      lsof +D "$bundle" >/dev/null 2>"$error_file"
      status=$?
      set -e
      if [ "$status" -eq 0 ]; then rm -f "$error_file"; return 0; fi
      if [ "$status" -eq 1 ] && [ ! -s "$error_file" ]; then
        rm -f "$error_file"
        return 1
      fi
      rm -f "$error_file"
      return 2
      ;;
    *) return 2 ;;
  esac
}

prune_bundle() {
  local name="$1" bundle current="" status
  printf '%s\n' "$name" | grep -Eq '^[0-9A-Za-z._-]+$' ||
    die 'prune requires one exact bundle name from --list-builds'
  bundle="$DEV_DATA_ROOT/$name"
  if ! recognized_bundle "$bundle" && ! legacy_bundle "$bundle"; then
    die "refusing unrecognized development bundle: $bundle"
  fi
  current="$(current_target 2>/dev/null || true)"
  [ "$bundle" != "$current" ] || die 'refusing to prune the current development bundle'
  set +e
  bundle_in_use "$bundle"
  status=$?
  set -e
  [ "$status" -ne 0 ] || die 'refusing to prune a bundle used by a running process'
  [ "$status" -ne 2 ] || die 'could not rule out a running reference; no bundle was deleted'
  log 'Warning: exact provider-hook references cannot be discovered reliably.'
  log "Removing explicitly selected inactive bundle: $bundle"
  rm -rf "$bundle"
}
