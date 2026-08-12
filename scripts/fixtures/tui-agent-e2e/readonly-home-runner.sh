#!/usr/bin/env bash
set -euo pipefail

home="$1"
shift
scratch="$1"
shift
install -d -m 0700 "$scratch/upper" "$scratch/work" "$scratch/home"
mount -t overlay overlay \
  -o "lowerdir=$home,upperdir=$scratch/upper,workdir=$scratch/work" \
  "$scratch/home"
mount --bind "$home" "$home"
mount -o remount,bind,ro "$home"
options="$(findmnt -n -o OPTIONS -T "$home")"
case ",$options," in
  *,ro,*) ;;
  *) printf '%s\n' 'home bind mount did not become read-only' >&2; exit 3 ;;
esac
export HOME="$scratch/home"
export XDG_CONFIG_HOME="$HOME/.config"
export XDG_STATE_HOME="$HOME/.local/state"
export XDG_DATA_HOME="$HOME/.local/share"
export XDG_CACHE_HOME="$HOME/.cache"
# The surrounding Codex task can carry an internal remote payload. A smoke
# process must never inherit anything that could become a model prompt.
unset CODEX_REMOTE_PAYLOAD CODEX_THREAD_ID CODEX_CI
case "${CODEX_HOME:-}" in
  "$home"/*) export CODEX_HOME="$HOME/${CODEX_HOME#"$home"/}" ;;
esac
case "${CLAUDE_CONFIG_DIR:-}" in
  "$home"/*) export CLAUDE_CONFIG_DIR="$HOME/${CLAUDE_CONFIG_DIR#"$home"/}" ;;
esac
exec "$@"
