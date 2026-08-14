#!/usr/bin/env python3
"""Interactive fixture for Blackpepper's Zellij terminal transparency checks."""

from __future__ import annotations

import base64
import os
import select
import signal
import sys
import termios
import tty


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
        elif command == "bell":
            sys.stdout.write("\a")
            sys.stdout.flush()
            print("BP_BELL_SENT", flush=True)
        elif command == "notify":
            sys.stdout.write("\033]9;BP_NOTIFICATION_E2E\a")
            sys.stdout.flush()
            print("BP_OSC9_SENT", flush=True)
        elif command == "focus":
            descriptor = sys.stdin.fileno()
            previous = termios.tcgetattr(descriptor)
            received = b""
            try:
                tty.setraw(descriptor)
                sys.stdout.write("\033[?1004hBP_FOCUS_READY\r\n")
                sys.stdout.flush()
                # Capture a short trailing window as well as the first event.
                # Stock Zellij 0.44.3 incorrectly turns one outer FocusOut
                # into FocusOut+FocusIn; stopping at three bytes would hide it.
                readable, _, _ = select.select([descriptor], [], [], 5.0)
                while readable and len(received) < 64:
                    chunk = os.read(descriptor, 64 - len(received))
                    if not chunk:
                        break
                    received += chunk
                    readable, _, _ = select.select([descriptor], [], [], 0.25)
            finally:
                sys.stdout.write("\033[?1004l")
                sys.stdout.flush()
                termios.tcsetattr(descriptor, termios.TCSADRAIN, previous)
            marker = "BP_FOCUS_INPUT" if received == b"\033[O" else "BP_FOCUS_UNEXPECTED"
            print(f"{marker}:{received.hex()}", flush=True)
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
