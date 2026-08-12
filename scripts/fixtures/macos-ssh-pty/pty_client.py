"""Small stdlib-only pseudo-terminal driver for the macOS SSH acceptance."""

from __future__ import annotations

import fcntl
import os
import pty
import select
import signal
import struct
import subprocess
import termios
import time


class PtyFailure(RuntimeError):
    pass


def set_size(descriptor: int, rows: int, columns: int) -> None:
    packed = struct.pack("HHHH", rows, columns, 0, 0)
    fcntl.ioctl(descriptor, termios.TIOCSWINSZ, packed)


class PtyClient:
    def __init__(self, command: list[str], rows: int, columns: int) -> None:
        self.master, slave = pty.openpty()
        set_size(slave, rows, columns)
        environment = os.environ.copy()
        environment.update({"TERM": "xterm-ghostty", "COLORTERM": "truecolor"})
        self.process = subprocess.Popen(
            command,
            stdin=slave,
            stdout=slave,
            stderr=slave,
            close_fds=True,
            env=environment,
        )
        os.close(slave)
        fcntl.fcntl(self.master, fcntl.F_SETFL, os.O_NONBLOCK)
        self.output = bytearray()

    def read(self, wait: float = 0.1) -> None:
        ready, _, _ = select.select([self.master], [], [], wait)
        if not ready:
            return
        try:
            chunk = os.read(self.master, 65536)
        except (BlockingIOError, OSError):
            return
        self.output.extend(chunk)

    def mark(self) -> int:
        self.read(0)
        return len(self.output)

    def wait_for(self, needle: bytes, start: int, timeout: float) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if needle in self.output[start:]:
                return
            if self.process.poll() is not None:
                raise PtyFailure(
                    f"PTY exited with status {self.process.returncode} "
                    f"while waiting for {needle!r}"
                )
            self.read()
        raise PtyFailure(f"timed out waiting for PTY output: {needle!r}")

    def send(self, data: bytes) -> None:
        os.write(self.master, data)

    def resize(self, rows: int, columns: int) -> None:
        set_size(self.master, rows, columns)
        os.kill(self.process.pid, signal.SIGWINCH)

    def wait_exit(self, timeout: float) -> int:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.read(0.1)
            result = self.process.poll()
            if result is not None:
                self.read(0)
                return result
        raise subprocess.TimeoutExpired(self.process.args, timeout)

    def close(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        os.close(self.master)
