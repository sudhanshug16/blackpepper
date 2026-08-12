#!/usr/bin/env python3
"""Deterministic provider process exercising Blackpepper's launch contracts."""

from __future__ import annotations

import json
import os
import pathlib
import re
import select
import subprocess
import sys
import tomllib
from typing import Any


REQUIRED_EVENTS = (
    "SessionStart",
    "UserPromptSubmit",
    "PermissionRequest",
    "PostToolUse",
    "Stop",
    "SessionEnd",
)


def fail(message: str) -> "NoReturn":
    print(f"blackpepper-agent-e2e shim: {message}", file=sys.stderr)
    raise SystemExit(2)


def atomic_json(path: pathlib.Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    temporary = path.with_suffix(f".tmp-{os.getpid()}")
    temporary.write_text(json.dumps(value, sort_keys=True), encoding="utf-8")
    temporary.chmod(0o600)
    temporary.replace(path)


def codex_contract(arguments: list[str]) -> dict[str, str]:
    commands: dict[str, str] = {}
    index = 0
    while index < len(arguments):
        if arguments[index] != "-c" or index + 1 >= len(arguments):
            index += 1
            continue
        parsed = tomllib.loads(arguments[index + 1])
        hooks = parsed.get("hooks", {})
        if len(hooks) != 1:
            fail("Codex hook override did not contain exactly one event")
        event, definitions = next(iter(hooks.items()))
        commands[event] = definitions[0]["hooks"][0]["command"]
        index += 2
    if set(commands) != set(REQUIRED_EVENTS):
        fail(f"Codex hook events differed: {tuple(commands)}")
    return {event: commands[event] for event in REQUIRED_EVENTS}


def claude_contract(arguments: list[str]) -> dict[str, str]:
    try:
        settings_path = pathlib.Path(arguments[arguments.index("--settings") + 1])
    except (ValueError, IndexError):
        fail("Claude did not receive --settings with a launch-scoped file")
    if not settings_path.is_absolute() or settings_path.stat().st_mode & 0o077:
        fail("Claude launch-scoped settings were not absolute and private")
    settings = json.loads(settings_path.read_text(encoding="utf-8"))
    hooks = settings.get("hooks", {})
    if set(hooks) != set(REQUIRED_EVENTS):
        fail(f"Claude hook events differed: {tuple(hooks)}")
    return {
        event: hooks[event][0]["hooks"][0]["command"]
        for event in REQUIRED_EVENTS
    }


def opencode_contract() -> tuple[dict[str, str], pathlib.Path]:
    try:
        inline = json.loads(os.environ["OPENCODE_CONFIG_CONTENT"])
        plugin_path = pathlib.Path(inline["plugin"][0])
    except (KeyError, IndexError, json.JSONDecodeError):
        fail("OpenCode inline config did not name one managed plugin")
    if not plugin_path.is_absolute() or plugin_path.stat().st_mode & 0o077:
        fail("OpenCode managed plugin was not absolute and private")
    source = plugin_path.read_text(encoding="utf-8")
    constants: dict[str, str] = {}
    for name in ("HELPER", "WORKSPACE", "RUN", "PANE"):
        match = re.search(rf"^const {name} = (.+);$", source, re.MULTILINE)
        if match is None:
            fail(f"OpenCode plugin omitted {name}")
        constants[name] = json.loads(match.group(1))
    required = ("Bun.spawn", "blackpepper.integration.ready", "semantic_sequence")
    if not all(fragment in source for fragment in required):
        fail("OpenCode plugin omitted its delivery or continuity contract")
    return constants, plugin_path


def hook_payload(event: str, secret: str, sequence: int | None = None) -> bytes:
    value: dict[str, Any] = {
        "hook_event_name": event,
        "type": event,
        "prompt": secret,
        "response": secret,
        "command": secret,
        "tool_content": secret,
        "terminal_text": secret,
    }
    if sequence is not None:
        value["semantic_sequence"] = sequence
    return json.dumps(value).encode()


class ProviderProcess:
    def __init__(self, provider: str, arguments: list[str]) -> None:
        self.provider = provider
        self.root = pathlib.Path(os.environ["BLACKPEPPER_AGENT_E2E_ROOT"])
        self.secret = os.environ["BLACKPEPPER_AGENT_E2E_SECRET"]
        self.run = os.environ["BLACKPEPPER_AGENT_RUN_ID"]
        self.workspace = os.environ["BLACKPEPPER_WORKSPACE_ID"]
        self.pane = os.environ["BLACKPEPPER_PANE_ID"]
        self.sequence = 0
        self.commands: dict[str, str] = {}
        self.helper: pathlib.Path | None = None
        self.contract = ""
        self.asset: pathlib.Path | None = None
        if provider == "codex":
            self.commands = codex_contract(arguments)
            self.contract = "codex-session-overrides"
        elif provider == "claude":
            self.commands = claude_contract(arguments)
            self.contract = "claude-launch-settings"
            self.asset = pathlib.Path(arguments[arguments.index("--settings") + 1])
        else:
            constants, self.asset = opencode_contract()
            expected = {"WORKSPACE": self.workspace, "RUN": self.run, "PANE": self.pane}
            if any(constants[key] != value for key, value in expected.items()):
                fail("OpenCode plugin IDs differed from the launch environment")
            self.helper = pathlib.Path(constants["HELPER"])
            self.contract = "opencode-managed-plugin"
        self.fifo = self.root / "controls" / f"{provider}-{self.run}.fifo"
        self.trace = self.root / "traces" / f"{provider}-{self.run}.jsonl"

    def invoke(self, event: str, semantic: bool = False) -> None:
        sequence = self.sequence if self.provider == "opencode" else None
        if semantic:
            self.sequence += 1
            sequence = self.sequence
        payload = hook_payload(event, self.secret, sequence)
        if self.provider == "opencode":
            command = [
                str(self.helper),
                "agent-event",
                "--provider",
                "opencode",
                "--workspace-id",
                self.workspace,
                "--run-id",
                self.run,
                "--pane-id",
                self.pane,
            ]
            completed = subprocess.run(command, input=payload, stdout=subprocess.DEVNULL,
                                       stderr=subprocess.DEVNULL, check=False)
        else:
            completed = subprocess.run(self.commands[event], input=payload, shell=True,
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                       check=False, env=os.environ)
        self.trace.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        with self.trace.open("a", encoding="utf-8") as trace:
            trace.write(json.dumps({"event": event, "sequence": sequence,
                                    "exit_code": completed.returncode}) + "\n")
        if completed.returncode != 0:
            fail(f"{event} hook did not preserve fail-silent exit status")

    def heartbeat(self) -> None:
        if self.provider != "opencode":
            return
        payload = hook_payload("blackpepper.integration.heartbeat", self.secret, self.sequence)
        command = [str(self.helper), "agent-event", "--provider", "opencode",
                   "--workspace-id", self.workspace, "--run-id", self.run,
                   "--pane-id", self.pane]
        subprocess.run(command, input=payload, stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL, check=False)

    def start(self) -> None:
        self.fifo.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.trace.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        if self.fifo.exists():
            self.fifo.unlink()
        os.mkfifo(self.fifo, 0o600)
        atomic_json(self.root / "runs" / f"{self.provider}.current", {
            "provider": self.provider,
            "run_id": self.run,
            "workspace_id": self.workspace,
            "pane_id": self.pane,
            "pid": os.getpid(),
            "fifo": str(self.fifo),
            "contract": self.contract,
            "asset": str(self.asset) if self.asset else None,
        })
        if self.provider == "opencode":
            self.invoke("blackpepper.integration.ready")
            self.invoke("session.created", semantic=True)
        else:
            self.invoke("SessionStart")
        print(f"Blackpepper {self.provider} acceptance provider {self.run}", flush=True)
        descriptor = os.open(self.fifo, os.O_RDWR | os.O_NONBLOCK)
        buffered = b""
        try:
            while True:
                readable, _, _ = select.select([descriptor], [], [], 1.0)
                if not readable:
                    self.heartbeat()
                    continue
                buffered += os.read(descriptor, 4096)
                while b"\n" in buffered:
                    line, buffered = buffered.split(b"\n", 1)
                    if self.handle(line.decode("ascii")):
                        return
        finally:
            os.close(descriptor)
            self.fifo.unlink(missing_ok=True)

    def handle(self, action: str) -> bool:
        events = {
            "codex": {"working": "UserPromptSubmit", "input": "PermissionRequest",
                      "done": "Stop", "unknown": "SessionEnd"},
            "claude": {"working": "UserPromptSubmit", "input": "PermissionRequest",
                       "done": "Stop", "unknown": "SessionEnd"},
            "opencode": {"working": "message.updated", "input": "permission.asked",
                         "done": "session.idle", "unknown": "session.error"},
        }
        if action == "exit":
            event = "session.deleted" if self.provider == "opencode" else "SessionEnd"
            self.invoke(event, semantic=self.provider == "opencode")
            return True
        event = events[self.provider].get(action)
        if event is None:
            fail(f"unsupported control action {action!r}")
        self.invoke(event, semantic=self.provider == "opencode")
        return False


def preflight(provider: str, arguments: list[str], root: pathlib.Path) -> bool:
    is_preflight = (provider == "codex" and arguments[-2:] == ["features", "list"]) or (
        provider == "claude" and arguments[-1:] == ["doctor"]
    )
    if not is_preflight:
        return False
    contract = codex_contract(arguments) if provider == "codex" else claude_contract(arguments)
    atomic_json(root / "preflight" / f"{provider}.json", {
        "provider": provider, "events": list(contract), "no_agent_started": True,
    })
    return True


def main() -> int:
    provider = pathlib.Path(sys.argv[0]).name
    if provider not in {"codex", "claude", "opencode"}:
        fail(f"unexpected shim name {provider!r}")
    root = pathlib.Path(os.environ["BLACKPEPPER_AGENT_E2E_ROOT"])
    if preflight(provider, sys.argv[1:], root):
        return 0
    ProviderProcess(provider, sys.argv[1:]).start()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
