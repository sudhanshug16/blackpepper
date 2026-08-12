#!/usr/bin/env python3
"""Start one real provider in a PTY without ever submitting a prompt."""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import pathlib
import pty
import select
import signal
import struct
import subprocess
import termios
import time


def provider_args(provider: str, binary: str, marker: pathlib.Path) -> list[str]:
    command = f"/usr/bin/printf '%s\\n' healthy > {marker}"
    if provider == "codex":
        definition = (
            "hooks.SessionStart=[{hooks=[{type=\"command\","
            f"command={json.dumps(command)},timeout=3}}]}}]"
        )
        return [binary, "-c", definition]
    settings = marker.with_suffix(".settings.json")
    settings.write_text(json.dumps({
        "hooks": {
            "SessionStart": [{
                "hooks": [{"type": "command", "command": command, "timeout": 3}]
            }]
        }
    }), encoding="utf-8")
    settings.chmod(0o600)
    return [binary, "--settings", str(settings)]


def run(provider: str, binary: str, marker: pathlib.Path) -> int:
    arguments = provider_args(provider, binary, marker)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
    process = subprocess.Popen(
        arguments,
        stdin=slave,
        stdout=slave,
        stderr=slave,
        close_fds=True,
        start_new_session=True,
    )
    os.close(slave)
    deadline = time.monotonic() + 12
    try:
        while time.monotonic() < deadline:
            if marker.read_text(encoding="ascii").strip() == "healthy" if marker.exists() else False:
                return 0
            if process.poll() is not None:
                return 2
            readable, _, _ = select.select([master], [], [], 0.1)
            if readable:
                try:
                    os.read(master, 65_536)
                except OSError:
                    pass
        return 2
    finally:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=2)
        os.close(master)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("provider", choices=("codex", "claude"))
    parser.add_argument("binary")
    parser.add_argument("marker", type=pathlib.Path)
    args = parser.parse_args()
    return run(args.provider, args.binary, args.marker)


if __name__ == "__main__":
    raise SystemExit(main())
