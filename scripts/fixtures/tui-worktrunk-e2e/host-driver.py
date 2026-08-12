#!/usr/bin/env python3
"""Drive real Worktrunk mutations through bp-host's JSON-lines protocol."""

from __future__ import annotations

import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import threading
import time

from host_protocol import (
    Helper,
    ProtocolFailure,
    approval,
    git_worktree_is_absent,
    host_payload,
    process_is_live,
    register,
    snapshot,
    wait_until,
)


def assert_setup_failed(helper: Helper, path: Path) -> None:
    record = next(
        (item for item in snapshot(helper)["workspaces"] if item["root_path"] == str(path)),
        None,
    )
    if record is None:
        raise AssertionError("setup-failed worktree was hidden from the host registry")
    setup = record.get("setup", {})
    if setup.get("status") != "failed" or "fixture setup failed" not in setup.get("message", ""):
        raise AssertionError(f"setup failure was not durable and actionable: {setup}")


def concurrent_lock(binary: str, build_id: str, repo: Path, marker: Path, release: Path) -> None:
    first = Helper(binary, build_id)
    preview = first.request(
        "worktrunk_create",
        {"repository_path": str(repo), "branch": "lock-first", "base": "main", "approval": None},
    )
    token = approval(preview)
    outcome: dict[str, object] = {}

    def mutate() -> None:
        try:
            outcome["payload"] = first.request(
                "worktrunk_create",
                {
                    "repository_path": str(repo),
                    "branch": "lock-first",
                    "base": "main",
                    "approval": token,
                },
            )
        except BaseException as error:  # surfaced on the main test thread
            outcome["error"] = error

    thread = threading.Thread(target=mutate, daemon=True)
    thread.start()
    wait_until("the guarded pre-start hook", marker.is_file)
    second = Helper(binary, build_id)
    started = time.monotonic()
    try:
        second.request(
            "worktrunk_create",
            {"repository_path": str(repo), "branch": "lock-second", "base": "main", "approval": None},
        )
        raise AssertionError("a concurrent Worktrunk mutation was accepted")
    except ProtocolFailure as error:
        if "Another Worktrunk mutation is active" not in str(error):
            raise
    if time.monotonic() - started > 3:
        raise AssertionError("concurrent Worktrunk refusal did not fail fast")
    second.close()
    release.touch()
    thread.join(timeout=20)
    if thread.is_alive():
        raise AssertionError("guarded Worktrunk mutation did not finish after release")
    if "error" in outcome:
        raise outcome["error"]  # type: ignore[misc]
    result = host_payload(outcome["payload"])  # type: ignore[arg-type]
    if result.get("kind") != "worktrunk_mutation" or result["outcome"].get("kind") != "switched":
        raise AssertionError(f"unexpected guarded mutation outcome: {result}")
    first.close()


def reconcile_lost_remove(
    binary: str,
    build_id: str,
    survivor: Path,
    target: Path,
    marker: Path,
    count: Path,
) -> None:
    helper = Helper(binary, build_id)
    survivor_id = register(helper, survivor, "lost-survivor")
    target_id = register(helper, target, "lost-target")
    token = approval(
        helper.request(
            "worktrunk_remove",
            {"workspace_id": target_id, "target_path": str(target), "approval": None},
        )
    )
    outcome: dict[str, object] = {}

    def remove() -> None:
        try:
            outcome["payload"] = helper.request(
                "worktrunk_remove",
                {"workspace_id": target_id, "target_path": str(target), "approval": token},
            )
        except BaseException as error:
            outcome["error"] = error

    thread = threading.Thread(target=remove, daemon=True)
    thread.start()
    wait_until("pre-remove hook dispatch", marker.is_file)
    observer = Helper(binary, build_id)
    pending = snapshot(observer)["pending_worktree_removals"]
    observer.close()
    if target_id not in pending:
        raise AssertionError("removal dispatch was not journaled before the response")
    registry = Path(os.environ["XDG_STATE_HOME"]) / "blackpepper/host-registry.sqlite3"
    database = sqlite3.connect(registry, timeout=1, isolation_level=None)
    database.execute("BEGIN IMMEDIATE")
    marker.with_suffix(".release").touch()
    wait_until("the real worktree removal", lambda: not target.exists())
    wait_until(
        "Git worktree metadata removal",
        lambda: git_worktree_is_absent(survivor, target),
    )
    if not thread.is_alive():
        raise AssertionError("remove response completed before loss could be exercised")
    helper.kill()
    thread.join(timeout=10)
    database.execute("ROLLBACK")
    database.close()
    if thread.is_alive():
        raise AssertionError("lost-response request reader did not stop")
    hook_pid = int(marker.read_text(encoding="utf-8").strip())
    wait_until("the lock guardian to contain the hook process", lambda: not process_is_live(hook_pid))

    recovery = Helper(binary, build_id)
    try:
        recovery.request(
            "worktrunk_remove",
            {"workspace_id": target_id, "target_path": str(target), "approval": token},
        )
        raise AssertionError("explicit retry bypassed the unknown-result journal")
    except ProtocolFailure as error:
        if "previous Worktrunk removal" not in str(error):
            raise

    deadline = time.monotonic() + 10
    while True:
        try:
            listed = host_payload(
                recovery.request(
                    "worktrunk_list",
                    {"workspace_id": target_id, "repository_path": str(target)},
                )
            )
            break
        except ProtocolFailure as error:
            if "Another Worktrunk mutation is active" not in str(error) or time.monotonic() >= deadline:
                raise
            time.sleep(0.1)
    if listed.get("kind") != "worktrees" or listed["list"].get("schema") != 2:
        raise AssertionError(f"reconciliation did not use authoritative schema 2: {listed}")
    state = snapshot(recovery)
    if target_id in state["pending_worktree_removals"]:
        raise AssertionError("authoritative list left the removal journal pending")
    if any(item["id"] == target_id for item in state["workspaces"]):
        raise AssertionError("authoritative list left a removed workspace ghost")
    if not any(item["id"] == survivor_id for item in state["workspaces"]):
        raise AssertionError("reconciliation removed the surviving workspace")
    lines = count.read_text(encoding="utf-8").splitlines()
    if lines != ["remove-dispatched"]:
        raise AssertionError(f"Worktrunk remove was dispatched more than once: {lines}")
    recovery.close()


def main() -> None:
    if len(sys.argv) != 10:
        raise SystemExit(
            "usage: host-driver.py BP_HOST WT SETUP_PATH LOCK_REPO LOCK_MARKER "
            "LOCK_RELEASE LOST_SURVIVOR LOST_TARGET LOST_MARKER"
        )
    binary, wt, setup_path, lock_repo, lock_marker, lock_release, survivor, target, lost_marker = sys.argv[1:]
    version = subprocess.check_output([binary, "--version"], text=True).strip()
    if not version.startswith("bp-host "):
        raise AssertionError(f"invalid bp-host version: {version}")
    build_id = version.removeprefix("bp-host ")
    if subprocess.check_output([wt, "--version"], text=True).strip() != "wt v0.72.0":
        raise AssertionError("acceptance did not resolve exact Worktrunk 0.72.0")
    inspector = Helper(binary, build_id)
    assert_setup_failed(inspector, Path(setup_path))
    inspector.close()
    concurrent_lock(binary, build_id, Path(lock_repo), Path(lock_marker), Path(lock_release))
    lost_marker_path = Path(lost_marker)
    reconcile_lost_remove(
        binary,
        build_id,
        Path(survivor),
        Path(target),
        lost_marker_path,
        lost_marker_path.with_suffix(".count"),
    )


if __name__ == "__main__":
    main()
