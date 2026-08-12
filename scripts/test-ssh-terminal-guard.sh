#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)"

python3 -B \
  "$ROOT/scripts/fixtures/ssh-terminal-guard/pty_scenario.py" \
  "$ROOT/scripts/ssh-terminal-guard.sh"

printf '%s\n' \
  'ssh terminal guard preserves status/termios and recovers after channel loss and signals'
