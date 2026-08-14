#!/usr/bin/env python3
"""Exercise bp-dev over the same macOS -> SSH PTY boundary used by Ghostty.

This is deliberately a protocol acceptance test, not UI automation. It opens a
new macOS pseudo-terminal, runs one isolated Blackpepper client on the selected
Linux host, and never reads or drives an existing Ghostty window.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(
    0, str(Path(__file__).resolve().parent / "fixtures" / "macos-ssh-pty")
)
from pty_client import PtyClient, PtyFailure  # noqa: E402
from remote_harness import (  # noqa: E402
    AcceptanceFailure,
    acceptance_zellij_version,
    cleanup_remote,
    create_remote_root,
    launch_command,
    require_macos,
)


DEFAULT_BP = "/home/dev/.local/bin/bp-dev"
INITIAL_SIZE = (44, 140)
RESIZED_SIZE = (30, 104)
OSC52 = b"\x1b]52;c;QlBfTUFDX0NMSVBCT0FSRF9PSw==\x07"


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run isolated macOS/Ghostty-compatible SSH PTY acceptance."
    )
    parser.add_argument(
        "target",
        help="Existing non-interactive SSH target, for example dev@devbox",
    )
    parser.add_argument(
        "--bp-path",
        default=DEFAULT_BP,
        help=f"Absolute bp-dev path on the Linux host (default: {DEFAULT_BP})",
    )
    parser.add_argument(
        "--timeout", type=float, default=45.0, help="Seconds per UI checkpoint"
    )
    parser.add_argument(
        "--artifacts",
        type=Path,
        help="Keep the raw PTY transcript at this local path even on success",
    )
    return parser.parse_args()


def size_after(
    client: PtyClient, label: bytes, start: int, timeout: float
) -> tuple[int, int]:
    marker = b"BP_MAC_SIZE_" + label + b":"
    client.wait_for(marker, start, timeout)
    deadline = time.monotonic() + timeout
    # Ratatui redraws only changed cells, so adjacent shell output can be
    # separated by cursor-positioning CSI sequences in the outer byte stream.
    csi = rb"(?:\x1b\[[0-?]*[ -/]*[@-~])*"
    pattern = re.compile(re.escape(marker) + rb"(\d+)" + csi + rb"\s*(\d+)")
    while time.monotonic() < deadline:
        match = pattern.search(client.output, start)
        if match:
            return int(match.group(1)), int(match.group(2))
        client.read()
    raise AcceptanceFailure("terminal size marker was incomplete")


def run_acceptance(client: PtyClient, timeout: float) -> None:
    # `bp` and `blackpepper` have different styles, so SGR bytes may separate
    # the two spans in the raw stream. The wordmark itself is one stable span.
    client.wait_for(b"blackpepper", 0, timeout)
    client.wait_for(b"HOSTS", 0, timeout)
    client.wait_for(b"SESSION", 0, timeout)
    client.wait_for(b"PORTS", 0, timeout)
    client.wait_for(b"workspace", 0, timeout)
    if b"\x1b[?1000h" not in client.output or b"\x1b[?1006h" not in client.output:
        raise AcceptanceFailure("Blackpepper did not enable SGR mouse input on the outer PTY")

    start = client.mark()
    client.send(b"\r")
    client.wait_for(b"blackpepper", start, timeout)
    time.sleep(0.4)
    client.read(0)

    start = client.mark()
    client.send(b"echo B\"\"P_MAC_INPUT_OK\r")
    client.wait_for(b"BP_MAC_INPUT_OK", start, timeout)
    start = client.mark()
    client.send(b"printf 'GHOSTTY_TERM:%s:%s\\n' \"$TERM\" \"$COLORTERM\"\r")
    client.wait_for(b"GHOSTTY_TERM:xterm-ghostty:truecolor", start, timeout)

    start = client.mark()
    client.send(b"printf 'BP_MAC_SIZE_BEFORE:'; stty size\r")
    before = size_after(client, b"BEFORE", start, timeout)
    client.resize(*RESIZED_SIZE)
    time.sleep(0.4)
    start = client.mark()
    client.send(b"printf 'BP_MAC_SIZE_AFTER:'; stty size\r")
    after = size_after(client, b"AFTER", start, timeout)
    if before == after or not (1 <= after[0] < RESIZED_SIZE[0] and 1 <= after[1] < RESIZED_SIZE[1]):
        raise AcceptanceFailure(f"embedded PTY did not resize plausibly: {before} -> {after}")

    start = client.mark()
    client.send(b"for i in $(seq -w 1 100); do echo BP_MAC_SCROLL_$i; done\r")
    client.wait_for(b"BP_MAC_SCROLL_100", start, timeout)
    start = client.mark()
    client.send(b"\x1b[<64;50;10M")
    client.wait_for(b"SCROLL", start, timeout)
    client.send(b"\x03")
    time.sleep(0.5)
    client.send(b"\r")
    time.sleep(0.3)
    start = client.mark()
    client.send(
        b"clear; echo GHOSTTY_MOUSE_EXIT_OK; "
        b"for i in $(seq -w 1 100); do echo BP_MAC_SCROLL_$i; done\r"
    )
    client.wait_for(b"GHOSTTY_MOUSE_EXIT_OK", start, timeout)
    client.wait_for(b"BP_MAC_SCROLL_100", start, timeout)

    start = client.mark()
    client.send(b"\x1b")
    client.wait_for(b"SCROLL:", start, timeout)
    time.sleep(0.2)
    start = client.mark()
    client.send(b"s")
    client.wait_for(b"ENTERING SEARCH TERM", start, timeout)
    client.send(b"BP_MAC_SCROLL_007\r")
    client.wait_for(b"SEARCHING", start, timeout)
    time.sleep(0.5)
    client.send(b"\x03")
    time.sleep(0.5)
    client.send(b"\r")
    time.sleep(0.3)
    start = client.mark()
    client.send(b"clear; echo VIOLET_SEARCH_EXIT_OK\r")
    client.wait_for(b"VIOLET_SEARCH_EXIT_OK", start, timeout)

    start = client.mark()
    client.send(b"printf '\\033]52;c;QlBfTUFDX0NMSVBCT0FSRF9PSw==\\a'\r")
    client.wait_for(OSC52, start, timeout)
    # Ratatui emits a cell-level diff, so cursor moves can appear between the
    # words. These fragments prove the concise, nonfatal handoff notice was
    # rendered without assuming a full-screen terminal parser.
    client.wait_for(b"Copy ", start, timeout)
    client.wait_for(b"sent ", start, timeout)
    client.wait_for(b"terminal.", start, timeout)

    start = client.mark()
    client.send(b"\x1d")
    client.wait_for(b" MANAGE ", start, timeout)

    start = client.mark()
    client.send(b":definitely-not-a-command\r")
    client.wait_for(b"Unknown", start, timeout)
    client.wait_for(b"command", start, timeout)
    client.send(b":quit\r")
    try:
        exit_status = client.wait_exit(15)
    except subprocess.TimeoutExpired as error:
        raise AcceptanceFailure("Blackpepper did not quit within 15 seconds") from error
    if exit_status != 0:
        raise AcceptanceFailure(f"SSH PTY exited with status {exit_status}")
    if b"\x1b[?1049l" not in client.output or b"\x1b[?1000l" not in client.output:
        raise AcceptanceFailure("Blackpepper did not restore the outer terminal modes on exit")


def main() -> int:
    args = arguments()
    root = ""
    zellij_version = ""
    client: PtyClient | None = None
    transcript: Path | None = args.artifacts
    try:
        zellij_version = acceptance_zellij_version()
        ghostty_version = require_macos()
        root, bp_version = create_remote_root(args.target, args.bp_path)
        client = PtyClient(
            launch_command(args.target, root, args.bp_path), *INITIAL_SIZE
        )
        run_acceptance(client, args.timeout)
        print(f"PASS: {ghostty_version}; {bp_version}")
        print(
            "PASS: macOS SSH PTY input, resize, mouse, scroll/search, "
            "OSC 52, errors, and cleanup"
        )
        return 0
    except (AcceptanceFailure, PtyFailure, subprocess.TimeoutExpired) as error:
        if transcript is None:
            transcript = Path("/tmp") / f"blackpepper-macos-ssh-pty-failure-{os.getpid()}.raw"
        print(f"FAIL: {error}", file=sys.stderr)
        print(f"Raw PTY transcript: {transcript}", file=sys.stderr)
        return 1
    finally:
        if transcript is not None and client is not None:
            transcript.parent.mkdir(parents=True, exist_ok=True)
            transcript.write_bytes(client.output)
        if client is not None:
            client.close()
        if root and zellij_version:
            cleanup_remote(args.target, root, zellij_version)


if __name__ == "__main__":
    raise SystemExit(main())
