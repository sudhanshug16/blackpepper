"""Minimal typed assertions for bp-host's JSON-lines acceptance protocol."""

from __future__ import annotations

import json
import os
from pathlib import Path
import signal
import subprocess
import time


class ProtocolFailure(RuntimeError):
    pass


class Helper:
    def __init__(self, binary: str, build_id: str) -> None:
        self.process = subprocess.Popen(
            [binary],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        self.next_id = 1
        response = self.request("handshake", {"client_version": build_id})
        if response.get("kind") != "handshake":
            raise AssertionError(f"unexpected handshake: {response}")

    def request(self, method: str, params: dict | None = None) -> dict:
        request = {
            "request_id": self.next_id,
            "protocol_version": 1,
            "method": method,
        }
        self.next_id += 1
        if params is not None:
            request["params"] = params
        assert self.process.stdin is not None
        assert self.process.stdout is not None
        self.process.stdin.write(json.dumps(request, separators=(",", ":")) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            raise ProtocolFailure("bp-host response was lost")
        response = json.loads(line)
        if response.get("status") != "ok":
            raise ProtocolFailure(response.get("error", {}).get("message", str(response)))
        return response["payload"]

    def close(self) -> None:
        if self.process.poll() is not None:
            return
        assert self.process.stdin is not None
        self.process.stdin.close()
        self.process.wait(timeout=10)
        if self.process.returncode != 0:
            assert self.process.stderr is not None
            raise AssertionError(self.process.stderr.read())

    def kill(self) -> None:
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGKILL)
            self.process.wait(timeout=10)


def host_payload(payload: dict) -> dict:
    if payload.get("kind") != "host_service":
        raise AssertionError(f"expected host-service payload, got: {payload}")
    return payload["payload"]


def register(helper: Helper, path: Path, name: str) -> str:
    payload = host_payload(
        helper.request(
            "register_workspace",
            {"root_path": str(path), "display_name": name},
        )
    )
    if payload.get("kind") != "workspace_registered":
        raise AssertionError(f"workspace registration failed: {payload}")
    return payload["workspace"]["id"]


def approval(payload: dict) -> dict:
    payload = host_payload(payload)
    if payload.get("kind") != "worktrunk_approval_required":
        raise AssertionError(f"expected approval preview, got: {payload}")
    command = payload["command"]
    forbidden = ("--force", "--force-delete", "--clobber", "--reap", "--no-hooks")
    if any(flag in command.split() for flag in forbidden):
        raise AssertionError(f"unsafe Worktrunk preview: {command}")
    return payload["approval"]


def wait_until(description: str, predicate, timeout: float = 15.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.05)
    raise AssertionError(f"timed out waiting for {description}")


def snapshot(helper: Helper) -> dict:
    payload = helper.request("snapshot")
    if payload.get("kind") != "snapshot":
        raise AssertionError(f"unexpected snapshot: {payload}")
    return payload["snapshot"]


def process_is_live(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True


def git_worktree_is_absent(repository: Path, target: Path) -> bool:
    output = subprocess.check_output(
        ["git", "-C", str(repository), "worktree", "list", "--porcelain"],
        text=True,
    )
    return f"worktree {target}\n" not in output
