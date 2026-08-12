#!/usr/bin/env python3
"""Interactive fixture for Blackpepper's Zellij terminal transparency checks."""

from __future__ import annotations

import base64
import os
import signal
import sys


COPY_TEXT = "blackpepper-osc52-e2e"
SIZE_LOG = os.environ.get("BP_TERMINAL_SIZE_LOG")


def report_size(_signal: int | None = None, _frame: object | None = None) -> None:
    size = os.get_terminal_size(sys.stdout.fileno())
    report = f"BP_TERMINAL_SIZE:{size.columns}x{size.lines}"
    if SIZE_LOG is not None:
        with open(SIZE_LOG, "a", encoding="utf-8") as size_log:
            size_log.write(f"{report}\n")
    print(report, flush=True)


def main() -> int:
    signal.signal(signal.SIGWINCH, report_size)
    for line in range(1, 151):
        print(f"BP_SCROLL_LINE_{line:03d}")
    report_size()
    print("BP_TERMINAL_READY", flush=True)

    for command in sys.stdin:
        command = command.strip()
        if command == "copy":
            payload = base64.b64encode(COPY_TEXT.encode()).decode()
            sys.stdout.write(f"\033]52;c;{payload}\a")
            sys.stdout.flush()
            print("BP_OSC52_SENT", flush=True)
        elif command == "size":
            report_size()
        elif command == "quit":
            print("BP_TERMINAL_DONE", flush=True)
            return 0
        else:
            print(f"BP_TERMINAL_UNKNOWN:{command}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
