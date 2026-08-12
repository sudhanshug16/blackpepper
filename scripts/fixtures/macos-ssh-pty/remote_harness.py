"""Remote setup for the isolated macOS-to-Linux PTY acceptance."""

from __future__ import annotations

import os
import shlex
import shutil
import subprocess
import sys


ROOT_PREFIX = "/tmp/blackpepper-macos-ssh-pty."


class AcceptanceFailure(RuntimeError):
    pass


def require_macos() -> str:
    if sys.platform != "darwin":
        raise AcceptanceFailure("this harness must run on macOS")
    ghostty = shutil.which("ghostty")
    if ghostty is None:
        candidate = "/Applications/Ghostty.app/Contents/MacOS/ghostty"
        if os.access(candidate, os.X_OK):
            ghostty = candidate
    if ghostty is None:
        raise AcceptanceFailure("Ghostty is not installed in /Applications or on PATH")
    version: list[str] = []
    for option in ("+version", "--version"):
        result = subprocess.run(
            [ghostty, option], capture_output=True, text=True, timeout=10, check=False
        )
        version = (result.stdout or result.stderr).splitlines()
        if result.returncode == 0 and version and version[0].startswith("Ghostty "):
            break
    if not version or not version[0].startswith("Ghostty "):
        raise AcceptanceFailure("could not read the installed Ghostty version")
    return version[0]


def ssh_base() -> list[str]:
    return [
        "/usr/bin/ssh",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "LogLevel=ERROR",
    ]


def remote(target: str, script: str, timeout: float = 20.0) -> str:
    command = ssh_base() + [target, "sh -lc " + shlex.quote(script)]
    result = subprocess.run(
        command, capture_output=True, text=True, timeout=timeout, check=False
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        raise AcceptanceFailure(
            f"SSH command failed with status {result.returncode}: {detail or 'no output'}"
        )
    return result.stdout.strip()


def create_remote_root(target: str, bp_path: str) -> tuple[str, str]:
    if not target or target.startswith("-") or any(c.isspace() for c in target):
        raise AcceptanceFailure("target must be one SSH host token without options")
    if not bp_path.startswith("/") or any(c in bp_path for c in "\r\n\0"):
        raise AcceptanceFailure("--bp-path must be one absolute remote path")
    quoted_bp = shlex.quote(bp_path)
    script = f"""
set -eu
test -x {quoted_bp} || {{ echo 'bp-dev is not executable: {quoted_bp}' >&2; exit 1; }}
version=$({quoted_bp} --version)
case "$version" in
  'blackpepper '*'-dev.'*) ;;
  *) echo "not a dev build: $version" >&2; exit 1 ;;
esac
infocmp xterm-ghostty >/dev/null 2>&1 || {{
  echo 'remote xterm-ghostty terminfo is missing' >&2; exit 1;
}}
root=$(mktemp -d {ROOT_PREFIX}XXXXXX)
chmod 700 "$root"
mkdir -m 700 "$root/state" "$root/run" "$root/config" "$root/cache" "$root/z" "$root/workspace"
mkdir -m 700 "$root/config/zellij"
printf '%s\\n' 'show_startup_tips false' 'show_release_notes false' \
  > "$root/config/zellij/config.kdl"
chmod 600 "$root/config/zellij/config.kdl"
printf '%s\\n%s\\n' "$root" "$version"
"""
    output = remote(target, script)
    lines = output.splitlines()
    if len(lines) != 2 or not lines[0].startswith(ROOT_PREFIX):
        raise AcceptanceFailure(f"remote setup returned an unexpected result: {output!r}")
    return lines[0], lines[1]


def cleanup_remote(target: str, root: str) -> None:
    if not root.startswith(ROOT_PREFIX):
        return
    quoted = shlex.quote(root)
    script = f"""
set -u
root={quoted}
zellij=$(find "$HOME/.local/share/blackpepper/sidecars/zellij/0.44.3" \
  -type f -name zellij -perm -u+x -print -quit 2>/dev/null || true)
if test -n "$zellij"; then
  ZELLIJ_SOCKET_DIR="$root/z" "$zellij" kill-all-sessions -y \
    >/dev/null 2>&1 || true
fi
rm -rf -- "$root"
"""
    try:
        remote(target, script)
    except (AcceptanceFailure, subprocess.TimeoutExpired):
        print(f"WARNING: isolated remote cleanup needs review: {root}", file=sys.stderr)


def launch_command(target: str, root: str, bp_path: str) -> list[str]:
    qroot, qbp = shlex.quote(root), shlex.quote(bp_path)
    script = f"""
set -eu
root={qroot}
cd "$root/workspace"
unset ZELLIJ_SOCKET_DIR
env XDG_STATE_HOME="$root/state" XDG_RUNTIME_DIR="$root/run" \
  XDG_CONFIG_HOME="$root/config" XDG_CACHE_HOME="$root/cache" \
  BLACKPEPPER_E2E_ZELLIJ_SOCKET_DIR="$root/z" ZELLIJ_CONFIG_DIR="$root/config/zellij" \
  TERM=xterm-ghostty COLORTERM=truecolor \
  SHELL=/bin/bash LANG=C.UTF-8 LC_ALL=C.UTF-8 {qbp}
"""
    return ssh_base() + ["-tt", target, "sh -lc " + shlex.quote(script)]
