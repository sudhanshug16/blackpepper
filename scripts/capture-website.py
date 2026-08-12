#!/usr/bin/env python3
"""Capture the final GitHub Pages view through Chrome DevTools."""

from __future__ import annotations

import argparse
import base64
import json
import os
import signal
import shutil
import socket
import struct
import subprocess
import tempfile
import time
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CHROME_COMMANDS = (
    "google-chrome", "google-chrome-stable", "chromium",
    "chromium-browser", "chrome", "chrome-headless-shell",
)
CAPTURES = ((1280, 720), (390, 844))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--chrome",
        type=Path,
        metavar="PATH",
        help="Chrome/Chromium executable. Defaults to BLACKPEPPER_CHROME, "
        "then a standard browser command on PATH.",
    )
    parser.add_argument(
        "--url",
        default=(ROOT / "docs/index.html").as_uri(),
        help="Page URL; defaults to the local docs/index.html",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=ROOT / "docs/assets",
    )
    return parser.parse_args()


def resolve_chrome(explicit: Path | None) -> Path:
    if explicit is not None:
        return require_executable(explicit.expanduser(), "--chrome")

    if configured := os.environ.get("BLACKPEPPER_CHROME"):
        return require_executable(Path(configured).expanduser(), "BLACKPEPPER_CHROME")

    for command in CHROME_COMMANDS:
        discovered = shutil.which(command)
        if discovered:
            return Path(discovered)

    raise SystemExit(
        "Chrome/Chromium was not found. Pass --chrome /absolute/path/to/chrome, "
        "set BLACKPEPPER_CHROME, or put one of these commands on PATH: "
        f"{', '.join(CHROME_COMMANDS)}."
    )


def require_executable(path: Path, source: str) -> Path:
    if path.is_file() and os.access(path, os.X_OK):
        return path
    raise SystemExit(
        f"{source} does not name an executable Chrome/Chromium file: {path}. "
        "Pass --chrome /absolute/path/to/chrome or set BLACKPEPPER_CHROME."
    )


class DevTools:
    def __init__(self, websocket_url: str) -> None:
        parsed = urllib.parse.urlparse(websocket_url)
        # Cold font rasterization can make a full-page screenshot take more
        # than five seconds on shared CI runners. Keep every call bounded while
        # allowing that legitimate work to finish.
        self.connection = socket.create_connection((parsed.hostname, parsed.port), timeout=30)
        key = base64.b64encode(os.urandom(16)).decode()
        request = (
            f"GET {parsed.path} HTTP/1.1\r\n"
            f"Host: {parsed.hostname}:{parsed.port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        )
        self.connection.sendall(request.encode())
        response = b""
        while b"\r\n\r\n" not in response:
            response += self.connection.recv(4096)
        if b" 101 " not in response.split(b"\r\n", 1)[0]:
            raise RuntimeError("Chrome refused the DevTools WebSocket upgrade")
        self.sequence = 0

    def close(self) -> None:
        self.connection.close()

    def call(self, method: str, params: dict | None = None) -> dict:
        self.sequence += 1
        expected = self.sequence
        payload = json.dumps(
            {"id": expected, "method": method, "params": params or {}}
        ).encode()
        mask = os.urandom(4)
        header = bytearray([0x81])
        if len(payload) < 126:
            header.append(0x80 | len(payload))
        elif len(payload) < 65536:
            header.extend([0x80 | 126])
            header.extend(struct.pack("!H", len(payload)))
        else:
            header.extend([0x80 | 127])
            header.extend(struct.pack("!Q", len(payload)))
        header.extend(mask)
        header.extend(
            bytes(byte.__xor__(mask[index % 4]) for index, byte in enumerate(payload))
        )
        self.connection.sendall(header)
        while True:
            message = self._receive()
            if message.get("id") != expected:
                continue
            if "error" in message:
                raise RuntimeError(str(message["error"]))
            return message.get("result", {})

    def _read_exact(self, length: int) -> bytes:
        result = b""
        while len(result) < length:
            chunk = self.connection.recv(length - len(result))
            if not chunk:
                raise EOFError("Chrome closed the DevTools connection")
            result += chunk
        return result

    def _receive(self) -> dict:
        while True:
            first, second = self._read_exact(2)
            opcode = first & 0x0F
            length = second & 0x7F
            if length == 126:
                length = struct.unpack("!H", self._read_exact(2))[0]
            elif length == 127:
                length = struct.unpack("!Q", self._read_exact(8))[0]
            payload = self._read_exact(length)
            if opcode == 1:
                return json.loads(payload)


def wait_for_port(profile: Path, process: subprocess.Popen[bytes]) -> int:
    marker = profile / "DevToolsActivePort"
    for _attempt in range(100):
        if marker.is_file():
            return int(marker.read_text().splitlines()[0])
        if process.poll() is not None:
            raise RuntimeError(f"Chrome exited before DevTools was ready ({process.returncode})")
        time.sleep(0.1)
    raise TimeoutError("Chrome DevTools did not become ready within 10 seconds")


def new_page(port: int, url: str) -> str:
    encoded = urllib.parse.quote(url, safe=":/")
    request = urllib.request.Request(
        f"http://127.0.0.1:{port}/json/new?{encoded}", method="PUT"
    )
    with urllib.request.urlopen(request, timeout=5) as response:
        return json.load(response)["webSocketDebuggerUrl"]


def capture(devtools: DevTools, url: str, output_dir: Path) -> None:
    devtools.call("Page.enable")
    devtools.call("Page.navigate", {"url": url})
    time.sleep(0.5)
    devtools.call(
        "Runtime.evaluate",
        {"expression": "document.fonts.ready", "awaitPromise": True},
    )
    for width, height in CAPTURES:
        devtools.call(
            "Emulation.setDeviceMetricsOverride",
            {
                "width": width,
                "height": height,
                "deviceScaleFactor": 1,
                "mobile": False,
            },
        )
        metrics = devtools.call(
            "Runtime.evaluate",
            {
                "expression": (
                    "({viewport: innerWidth, content: "
                    "document.documentElement.scrollWidth})"
                ),
                "returnByValue": True,
            },
        )["result"]["value"]
        if metrics["content"] > metrics["viewport"]:
            raise RuntimeError(
                f"{width} px viewport overflows to {metrics['content']} px"
            )
        result = devtools.call(
            "Page.captureScreenshot",
            {
                "format": "png",
                "fromSurface": True,
                "captureBeyondViewport": False,
                "clip": {
                    "x": 0,
                    "y": 0,
                    "width": width,
                    "height": height,
                    "scale": 1,
                },
            },
        )
        destination = output_dir / f"site-{width}x{height}.png"
        destination.write_bytes(base64.b64decode(result["data"]))
        print(f"captured {destination} ({width}x{height}, no horizontal overflow)")


def stop_chrome(process: subprocess.Popen[bytes]) -> None:
    """Stop the exact Chrome process group before removing its profile."""
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait(timeout=5)


def remove_profile(directory: Path) -> None:
    # Chrome's helper processes can close their last profile files just after
    # the browser exits. Retry that bounded race instead of failing a capture
    # whose pixels and overflow checks already completed successfully.
    for _attempt in range(50):
        try:
            shutil.rmtree(directory)
            return
        except FileNotFoundError:
            return
        except OSError:
            time.sleep(0.1)
    shutil.rmtree(directory)


def main() -> None:
    args = parse_args()
    chrome = resolve_chrome(args.chrome)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    directory = Path(tempfile.mkdtemp(prefix="blackpepper-site-capture-"))
    try:
        profile = directory / "profile"
        process = subprocess.Popen(
            [
                str(chrome),
                "--headless=new",
                "--no-sandbox",
                "--disable-gpu",
                "--disable-dev-shm-usage",
                "--disable-background-networking",
                "--remote-debugging-port=0",
                f"--user-data-dir={profile}",
                "about:blank",
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        try:
            port = wait_for_port(profile, process)
            devtools = DevTools(new_page(port, args.url))
            try:
                capture(devtools, args.url, args.output_dir)
            finally:
                devtools.close()
        finally:
            stop_chrome(process)
    finally:
        remove_profile(directory)


if __name__ == "__main__":
    main()
