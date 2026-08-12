"""Shared PTY and fake-SSH support for the terminal-guard acceptance test."""

from __future__ import annotations

import fcntl
import json
import os
import select
import signal
import subprocess
import sys
import termios
import time
from pathlib import Path

ENABLE = b"\x1b[?1049h\x1b[?25l\x1b[?2004h\x1b[?1000h\x1b[?1006h"
RESET = (
    b"\x1b[?1l\x1b>\x1b[?2004l"
    b"\x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l"
    b"\x1b[?1005l\x1b[?1006l\x1b[0m\x1b[?1049l\x1b[?25h"
)
TIMEOUT = 8.0
CALLER_COMMAND = (
    'before=$(stty -g < /dev/tty) || exit 90; '
    '"$1" "$2" "$3"; rc=$?; '
    'after=$(stty -g < /dev/tty) || exit 91; '
    'if [ "$before" = "$after" ]; then match=yes; else match=no; fi; '
    'printf "\\nGUARD_RESULT:%s\\nTTY_MATCH:%s\\n" "$rc" "$match"; '
    'printf "TTY_BEFORE:%s\\nTTY_AFTER:%s\\n" "$before" "$after"; '
    'IFS= read -r probe; printf "CALLER_SHELL:%s\\n" "$probe"'
)


class ScenarioFailure(RuntimeError):
    pass


def begin_detached_session() -> None:
    """Start an isolated session with normal lifecycle-signal dispositions."""
    # Test runners commonly ignore QUIT. A shell cannot trap a signal that was
    # ignored on entry, so start the isolated caller with normal dispositions.
    for caught in (
        signal.SIGHUP,
        signal.SIGINT,
        signal.SIGQUIT,
        signal.SIGTERM,
    ):
        signal.signal(caught, signal.SIG_DFL)
    os.setsid()


def claim_controlling_terminal() -> None:
    """Make fd 0's fresh PTY the child's controlling terminal."""
    begin_detached_session()
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)


def fake_ssh() -> int:
    Path(os.environ["GUARD_ARGS"]).write_text(json.dumps(sys.argv[1:]))
    with Path(os.environ["GUARD_CALLS"]).open("a") as calls:
        calls.write("called\n")

    tty = os.open("/dev/tty", os.O_RDWR)
    attributes = termios.tcgetattr(tty)
    attributes[3] &= ~(termios.ECHO | termios.ICANON)
    attributes[6][termios.VMIN] = 1
    attributes[6][termios.VTIME] = 0
    termios.tcsetattr(tty, termios.TCSANOW, attributes)
    os.write(tty, ENABLE)

    # A non-interactive shell may start its asynchronous child with INT and
    # QUIT ignored. Reset every exercised signal before advertising readiness.
    for caught in (
        signal.SIGHUP,
        signal.SIGINT,
        signal.SIGQUIT,
        signal.SIGTERM,
    ):
        signal.signal(caught, signal.SIG_DFL)
    Path(os.environ["GUARD_READY"]).write_text(
        f"{os.getpid()} {os.getppid()}\n"
    )

    if os.environ["GUARD_MODE"] == "drop":
        descriptor = int(os.environ["GUARD_DROP_FD"])
        while os.read(descriptor, 4096):
            pass
        return 255

    while True:
        signal.pause()


def read_available(master: int, output: bytearray, wait: float = 0.05) -> None:
    ready, _, _ = select.select([master], [], [], wait)
    if not ready:
        return
    try:
        chunk = os.read(master, 65536)
    except OSError:
        return
    output.extend(chunk)


def wait_for_file(path: Path, process: subprocess.Popen[bytes], master: int) -> bytes:
    output = bytearray()
    deadline = time.monotonic() + TIMEOUT
    while time.monotonic() < deadline:
        read_available(master, output)
        if path.exists():
            return bytes(output)
        if process.poll() is not None:
            raise ScenarioFailure(
                f"guard exited {process.returncode} before fake ssh became ready"
            )
    raise ScenarioFailure("fake ssh did not become ready")


def collect(process: subprocess.Popen[bytes], master: int, initial: bytes) -> bytes:
    output = bytearray(initial)
    deadline = time.monotonic() + TIMEOUT
    while time.monotonic() < deadline:
        read_available(master, output)
        if process.poll() is not None:
            for _ in range(3):
                read_available(master, output, 0)
            return bytes(output)
    process.kill()
    process.wait(timeout=2)
    raise ScenarioFailure("guard did not exit before the timeout")


def wait_for_marker(
    process: subprocess.Popen[bytes],
    master: int,
    initial: bytes,
    marker: bytes,
) -> bytes:
    output = bytearray(initial)
    deadline = time.monotonic() + TIMEOUT
    while marker not in output and time.monotonic() < deadline:
        read_available(master, output)
        if process.poll() is not None:
            break
    if marker not in output:
        raise ScenarioFailure(f"caller did not emit {marker!r}")
    return bytes(output)


def environment(root: Path, mode: str, drop_fd: int) -> dict[str, str]:
    value = os.environ.copy()
    value.update(
        {
            "PATH": f"{root / 'bin'}:/usr/bin:/bin",
            "GUARD_ARGS": str(root / f"{mode}.args"),
            "GUARD_CALLS": str(root / f"{mode}.calls"),
            "GUARD_DROP_FD": str(drop_fd),
            "GUARD_MODE": mode,
            "GUARD_READY": str(root / f"{mode}.ready"),
        }
    )
    return value


def assert_common(root: Path, mode: str, output: bytes) -> None:
    if output.find(ENABLE) == -1 or output.rfind(RESET) < output.find(ENABLE):
        raise ScenarioFailure(f"{mode}: terminal reset did not follow enabled modes")
    calls = (root / f"{mode}.calls").read_text().splitlines()
    if calls != ["called"]:
        raise ScenarioFailure(f"{mode}: fake ssh invocation count was {len(calls)}")


def assert_termios_restored(mode: str, output: bytes) -> None:
    if b"TTY_MATCH:yes" in output:
        return

    values: dict[str, str] = {}
    for line in output.decode(errors="replace").splitlines():
        for label in ("TTY_BEFORE:", "TTY_AFTER:"):
            if line.startswith(label):
                values[label] = line.removeprefix(label)
    before = values.get("TTY_BEFORE:")
    after = values.get("TTY_AFTER:")

    # Darwin sets the transient PENDIN state bit when canonical mode is
    # restored. tcsetattr cannot clear it; compare every persistent setting.
    if sys.platform == "darwin" and before and after:

        def without_pendin(value: str) -> str:
            fields = value.split(":")
            for index, field in enumerate(fields):
                if field.startswith("lflag="):
                    flags = int(field.removeprefix("lflag="), 16)
                    fields[index] = f"lflag={flags & ~termios.PENDIN:x}"
            return ":".join(fields)

        if without_pendin(before) == without_pendin(after):
            return

    raise ScenarioFailure(
        f"{mode}: exact persistent caller termios was not restored: {output!r}"
    )
