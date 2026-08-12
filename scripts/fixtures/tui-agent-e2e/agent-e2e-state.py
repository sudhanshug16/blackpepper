#!/usr/bin/env python3
"""Drive a fake provider and assert the isolated host's durable agent state."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import sqlite3
import sys
import time
from typing import Any


def event_database(root: pathlib.Path) -> pathlib.Path:
    name = os.environ.get("BLACKPEPPER_AGENT_E2E_DB", "agent-events.sqlite3")
    if name not in {"agent-events.sqlite3", "agent-events-dev.sqlite3"}:
        raise AssertionError(f"unexpected agent event database name: {name!r}")
    return root.parent / "state" / "blackpepper" / name


def load_meta(root: pathlib.Path, provider: str) -> dict[str, Any]:
    path = root / "runs" / f"{provider}.current"
    return json.loads(path.read_text(encoding="utf-8"))


def wait_meta(root: pathlib.Path, provider: str, timeout: float) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            return load_meta(root, provider)
        except (FileNotFoundError, json.JSONDecodeError):
            time.sleep(0.05)
    raise AssertionError(f"timed out waiting for {provider} launch metadata")


def send_control(root: pathlib.Path, provider: str, action: str, timeout: float) -> None:
    meta = wait_meta(root, provider, timeout)
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            descriptor = os.open(meta["fifo"], os.O_WRONLY | os.O_NONBLOCK)
            try:
                os.write(descriptor, f"{action}\n".encode("ascii"))
            finally:
                os.close(descriptor)
            return
        except (FileNotFoundError, OSError):
            time.sleep(0.05)
    raise AssertionError(f"timed out sending {action!r} to {provider}")


def read_state(database: pathlib.Path, run_id: str) -> dict[str, Any] | None:
    try:
        connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True, timeout=0.2)
        snapshot_row = connection.execute(
            "SELECT latest_snapshot_json FROM agent_status_runs WHERE run_id = ?", (run_id,)
        ).fetchone()
        context_row = connection.execute(
            "SELECT active FROM agent_run_context WHERE run_id = ?", (run_id,)
        ).fetchone()
        if snapshot_row is None or context_row is None:
            return None
        snapshot = json.loads(snapshot_row[0])
        sequence = snapshot.get("last_event_sequence")
        event_row = connection.execute(
            "SELECT event_json FROM agent_status_events WHERE run_id = ? AND sequence = ?",
            (run_id, sequence),
        ).fetchone()
        event = json.loads(event_row[0]) if event_row else None
        return {"snapshot": snapshot, "event": event, "active": bool(context_row[0])}
    except (sqlite3.Error, FileNotFoundError, json.JSONDecodeError):
        return None
    finally:
        if "connection" in locals():
            connection.close()


def wait_state(
    root: pathlib.Path,
    provider: str,
    state: str,
    source: str,
    kind: str,
    active: bool,
    timeout: float,
) -> None:
    meta = wait_meta(root, provider, timeout)
    database = event_database(root)
    deadline = time.monotonic() + timeout
    last: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        last = read_state(database, meta["run_id"])
        if last is not None:
            event_kind = (last["event"] or {}).get("kind", {}).get("type")
            if (
                last["snapshot"].get("state") == state
                and (last["event"] or {}).get("source") == source
                and event_kind == kind
                and last["active"] is active
            ):
                health = last["snapshot"].get("integration_health", {}).get("status")
                if state != "exited" and health != "healthy":
                    raise AssertionError(f"{provider} integration was not healthy: {last}")
                return
        time.sleep(0.05)
    raise AssertionError(
        f"timed out waiting for {provider}: state={state} source={source} "
        f"kind={kind} active={active}; last={last}"
    )


def assert_contract(root: pathlib.Path, provider: str) -> None:
    meta = wait_meta(root, provider, 10)
    expected = {
        "codex": "codex-session-overrides",
        "claude": "claude-launch-settings",
        "opencode": "opencode-managed-plugin",
    }[provider]
    if meta.get("contract") != expected:
        raise AssertionError(f"unexpected {provider} contract: {meta}")
    if provider in {"codex", "claude"}:
        preflight = root / "preflight" / f"{provider}.json"
        value = json.loads(preflight.read_text(encoding="utf-8"))
        if value.get("events") != list((
            "SessionStart", "UserPromptSubmit", "PermissionRequest",
            "PostToolUse", "Stop", "SessionEnd",
        )) or value.get("no_agent_started") is not True:
            raise AssertionError(f"unexpected {provider} preflight: {value}")
    asset = meta.get("asset")
    if provider != "codex":
        path = pathlib.Path(asset)
        if not path.is_file() or path.stat().st_mode & 0o077:
            raise AssertionError(f"managed {provider} asset is absent or not 0600: {asset}")


def assert_redacted(root: pathlib.Path, secret: str) -> None:
    database = event_database(root)
    for path in database.parent.glob(f"{database.name}*"):
        if secret.encode() in path.read_bytes():
            raise AssertionError(f"sensitive provider payload persisted in {path}")
    forbidden = ("prompt", "response", "command", "tool_content", "terminal_text")
    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    try:
        for table, column in (
            ("agent_status_events", "event_json"),
            ("agent_status_events", "snapshot_json"),
            ("agent_status_runs", "latest_snapshot_json"),
        ):
            for (encoded,) in connection.execute(f"SELECT {column} FROM {table}"):
                lowered = encoded.lower()
                if any(term in lowered for term in forbidden):
                    raise AssertionError(f"forbidden payload field persisted in {table}.{column}")
    finally:
        connection.close()


def field(root: pathlib.Path, provider: str, name: str) -> None:
    value = wait_meta(root, provider, 10).get(name)
    print("" if value is None else value)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=pathlib.Path, required=True)
    subparsers = parser.add_subparsers(dest="command", required=True)
    meta_parser = subparsers.add_parser("wait-meta")
    meta_parser.add_argument("provider")
    control_parser = subparsers.add_parser("control")
    control_parser.add_argument("provider")
    control_parser.add_argument("action")
    state_parser = subparsers.add_parser("wait-state")
    state_parser.add_argument("provider")
    state_parser.add_argument("state")
    state_parser.add_argument("source")
    state_parser.add_argument("kind")
    state_parser.add_argument("active", choices=("true", "false"))
    contract_parser = subparsers.add_parser("assert-contract")
    contract_parser.add_argument("provider")
    field_parser = subparsers.add_parser("field")
    field_parser.add_argument("provider")
    field_parser.add_argument("name")
    redacted_parser = subparsers.add_parser("assert-redacted")
    redacted_parser.add_argument("secret")
    args = parser.parse_args()
    if args.command == "wait-meta":
        wait_meta(args.root, args.provider, 20)
    elif args.command == "control":
        send_control(args.root, args.provider, args.action, 10)
    elif args.command == "wait-state":
        wait_state(args.root, args.provider, args.state, args.source, args.kind,
                   args.active == "true", 20)
    elif args.command == "assert-contract":
        assert_contract(args.root, args.provider)
    elif args.command == "field":
        field(args.root, args.provider, args.name)
    else:
        assert_redacted(args.root, args.secret)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, KeyError, OSError, json.JSONDecodeError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        raise SystemExit(1) from error
