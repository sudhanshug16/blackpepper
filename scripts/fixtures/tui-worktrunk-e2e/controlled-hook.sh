#!/usr/bin/env bash
set -euo pipefail

mode="${1:-}"
shift || true

case "$mode" in
  setup)
    [ "$#" -eq 2 ] || exit 64
    branch="$1"
    root="$2"
    printf '%s\n' "$branch" > "$root/setup-${branch}.ran"
    if [ "$branch" = setup-fails ]; then
      printf '%s\n' 'fixture setup failed intentionally' >&2
      exit 23
    fi
    ;;
  hold)
    [ "$#" -eq 2 ] || exit 64
    marker="$1"
    release="$2"
    printf '%s\n' "$$" > "$marker"
    for _attempt in $(seq 1 600); do
      [ ! -e "$release" ] || exit 0
      sleep 0.05
    done
    printf '%s\n' 'fixture hold timed out' >&2
    exit 70
    ;;
  remove-gate)
    [ "$#" -eq 3 ] || exit 64
    marker="$1"
    release="$2"
    count="$3"
    printf '%s\n' 'remove-dispatched' >> "$count"
    printf '%s\n' "$$" > "$marker"
    for _attempt in $(seq 1 600); do
      [ ! -e "$release" ] || exit 0
      sleep 0.05
    done
    printf '%s\n' 'fixture remove hold timed out' >&2
    exit 71
    ;;
  *)
    printf 'unknown controlled hook mode: %s\n' "$mode" >&2
    exit 64
    ;;
esac
