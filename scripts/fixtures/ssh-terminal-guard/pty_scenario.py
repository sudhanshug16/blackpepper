#!/usr/bin/env python3
"""Hermetic caller-PTY checks for ssh-terminal-guard.sh."""

from __future__ import annotations

import json
import os
import pty
import shutil
import signal
import subprocess
import sys
import tempfile
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))

from pty_test_lib import (  # noqa: E402
    CALLER_COMMAND,
    RESET,
    ScenarioFailure,
    assert_common,
    assert_termios_restored,
    begin_detached_session,
    claim_controlling_terminal,
    collect,
    environment,
    fake_ssh,
    wait_for_file,
    wait_for_marker,
)


def no_controlling_tty(root: Path, guard: Path) -> None:
    mode = "no-tty"
    process = subprocess.run(
        [str(guard), "must-not-run"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
        env=environment(root, mode, -1),
        preexec_fn=begin_detached_session,
    )

    if process.returncode != 1:
        raise ScenarioFailure(
            f"{mode}: expected status 1, got {process.returncode}"
        )
    if b"/dev/tty is unavailable" not in process.stderr:
        raise ScenarioFailure(f"{mode}: actionable error was not reported")
    if (root / f"{mode}.calls").exists():
        raise ScenarioFailure(f"{mode}: fake ssh was invoked")


def missing_ssh(root: Path, guard: Path) -> None:
    bin_dir = root / "no-ssh-bin"
    bin_dir.mkdir()
    stty = shutil.which("stty")
    if stty is None:
        raise ScenarioFailure("missing: stty is unavailable")
    (bin_dir / "stty").symlink_to(stty)

    master, slave = pty.openpty()
    env = os.environ.copy()
    env["PATH"] = str(bin_dir)
    process = subprocess.Popen(
        [
            "/bin/sh",
            "-c",
            CALLER_COMMAND,
            "guard-caller",
            str(guard),
            "missing-host",
            "",
        ],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        close_fds=True,
        env=env,
        preexec_fn=claim_controlling_terminal,
    )
    initial = wait_for_marker(process, master, b"", b"GUARD_RESULT:127")
    os.write(master, b"still-usable\n")
    transcript = collect(process, master, initial)
    os.close(master)
    os.close(slave)

    if process.returncode != 0:
        raise ScenarioFailure(f"missing: caller exited {process.returncode}")
    if b"system ssh was not found on PATH" not in transcript:
        raise ScenarioFailure("missing: actionable error was not reported")
    if RESET not in transcript:
        raise ScenarioFailure("missing: terminal reset was not written")
    assert_termios_restored("missing", transcript)
    if b"CALLER_SHELL:still-usable" not in transcript:
        raise ScenarioFailure("missing: caller shell did not remain usable")


def network_loss(root: Path, guard: Path) -> None:
    master, slave = pty.openpty()
    drop_read, drop_write = os.pipe()
    process = subprocess.Popen(
        [
            "/bin/sh",
            "-c",
            CALLER_COMMAND,
            "guard-caller",
            str(guard),
            "test-host",
            "exec bp-dev",
        ],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        close_fds=True,
        pass_fds=(drop_read,),
        env=environment(root, "drop", drop_read),
        preexec_fn=claim_controlling_terminal,
    )
    initial = wait_for_file(root / "drop.ready", process, master)
    os.close(drop_write)
    os.close(drop_read)

    output = wait_for_marker(process, master, initial, b"TTY_AFTER:")
    assert_termios_restored("drop", output)
    if b"GUARD_RESULT:255" not in output:
        raise ScenarioFailure("drop: ssh exit status 255 was not preserved")
    os.write(master, b"still-usable\n")
    transcript = collect(process, master, output)
    os.close(master)
    os.close(slave)

    if b"CALLER_SHELL:still-usable" not in transcript:
        raise ScenarioFailure("drop: caller shell did not remain usable")
    assert_common(root, "drop", transcript)
    if json.loads((root / "drop.args").read_text()) != [
        "test-host",
        "exec bp-dev",
    ]:
        raise ScenarioFailure("drop: ssh arguments changed")


def caught_signal(root: Path, guard: Path, caught: signal.Signals) -> None:
    name = caught.name.lower()
    master, slave = pty.openpty()
    drop_read, drop_write = os.pipe()
    process = subprocess.Popen(
        [
            "/bin/sh",
            "-c",
            CALLER_COMMAND,
            "guard-caller",
            str(guard),
            "signal-host",
            "",
        ],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        close_fds=True,
        pass_fds=(drop_read,),
        env=environment(root, name, drop_read),
        preexec_fn=claim_controlling_terminal,
    )
    initial = wait_for_file(root / f"{name}.ready", process, master)
    _, wrapper_pid = (root / f"{name}.ready").read_text().split()
    os.kill(int(wrapper_pid), caught)
    expected = 128 + caught.value
    output = wait_for_marker(
        process, master, initial, f"GUARD_RESULT:{expected}".encode()
    )
    os.write(master, b"still-usable\n")
    transcript = collect(process, master, output)
    os.close(drop_read)
    os.close(drop_write)
    os.close(master)
    os.close(slave)

    if process.returncode != 0:
        raise ScenarioFailure(f"{name}: caller exited {process.returncode}")
    assert_termios_restored(name, transcript)
    if b"CALLER_SHELL:still-usable" not in transcript:
        raise ScenarioFailure(f"{name}: caller shell did not remain usable")
    assert_common(root, name, transcript)


def run(guard: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="blackpepper-ssh-guard.") as value:
        root = Path(value)
        (root / "bin").mkdir()
        (root / "bin" / "ssh").symlink_to(Path(__file__).resolve())
        no_controlling_tty(root, guard)
        missing_ssh(root, guard)
        network_loss(root, guard)
        for caught in (
            signal.SIGHUP,
            signal.SIGINT,
            signal.SIGQUIT,
            signal.SIGTERM,
        ):
            caught_signal(root, guard, caught)


if __name__ == "__main__":
    if Path(sys.argv[0]).name == "ssh":
        raise SystemExit(fake_ssh())
    if len(sys.argv) != 2:
        raise SystemExit("usage: pty_scenario.py PATH_TO_GUARD")
    try:
        run(Path(sys.argv[1]).resolve())
    except ScenarioFailure as error:
        print(f"FAIL: {error}", file=sys.stderr)
        raise SystemExit(1) from error
