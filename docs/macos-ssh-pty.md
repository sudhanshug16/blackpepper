# macOS SSH PTY acceptance

## Recommended SSH boundary

Run the native macOS or Linux `bp` client locally and connect with `:host
connect`. That keeps the process which owns raw mode and the alternate screen
on the same machine as the terminal emulator. A managed remote SSH channel can
then disappear without taking away Blackpepper's path to restore the terminal.

Running Linux `bp` inside an outer `ssh` session is supported for development,
but it has a hard transport limit: after that SSH data channel disappears, the
remote process cannot send its final terminal-reset bytes to the local
emulator. OpenSSH restores its own local termios on handled exits, but it has no
post-session terminal-reset option. `LocalCommand` runs after connection, not
after disconnection.

For that outer-SSH workflow, run the non-installed support wrapper from a
checkout:

```sh
scripts/ssh-terminal-guard.sh -t \
  dev@sudhanshu-devbox /home/dev/.local/bin/bp-dev
```

The wrapper snapshots the exact `/dev/tty` termios value, invokes the system
`ssh`, then restores that value and writes a conservative DEC mode reset
directly to the caller's `/dev/tty`. It preserves SSH's exit status and handles
`HUP`, `INT`, `QUIT`, and `TERM` without signaling a process group. It
deliberately does not issue a full terminal reset, so primary-screen history is
preserved. It refuses to start SSH when `/dev/tty` is unavailable because it
could not guarantee recovery in that environment.
No process can recover from local `SIGKILL`, terminal-emulator failure, or a
machine crash; Ghostty's **Reset** action is the final manual recovery and may
clear visible history.

If the terminal is already stranded and no guard snapshot exists, run this in
the affected local shell (typing may not echo):

```sh
stty sane </dev/tty
{
  printf '\033[?1l\033>\033[?2004l'
  printf '\033[?9l\033[?1000l\033[?1002l\033[?1003l'
  printf '\033[?1005l\033[?1006l\033[0m\033[?1049l\033[?25h'
} >/dev/tty
```

This avoids the full RIS reset and therefore preserves primary-screen history,
but `stty sane` cannot restore custom termios choices that were never captured.
Ghostty's **Reset** action is stronger and may clear visible history.

`scripts/test-macos-ssh-pty-e2e.py` checks the terminal protocol Blackpepper
uses when Ghostty on a Mac is already SSH'd into a Linux workspace host. It is
a repeatable acceptance test, not remote-control automation.

Run it from a macOS checkout with an existing non-interactive SSH target:

```sh
python3 scripts/test-macos-ssh-pty-e2e.py \
  dev@sudhanshu-devbox \
  --bp-path /home/dev/.local/bin/bp-dev
```

The harness fails before launching Blackpepper when any prerequisite is
missing. It requires:

- macOS and an installed Ghostty whose version can be read;
- `/usr/bin/ssh` access that succeeds with `BatchMode=yes`;
- the exact executable `bp-dev` path on a Linux host;
- `xterm-ghostty` terminfo on that host; and
- Python 3 from the macOS base system. No Python package is needed.

## Isolation

The harness opens a new macOS pseudo-terminal and a new SSH connection. It
does not inspect, type into, resize, or close an existing Ghostty window. It
sets Blackpepper's registry, runtime, cache, config, and Zellij socket paths to
a fresh `0700` directory named `/tmp/blackpepper-macos-ssh-pty.*` on the Linux
host. Its Zellij config disables first-run tips only inside that directory.

On success or failure, the harness terminates only sessions under that socket
tree and removes that exact temporary directory. Existing Blackpepper and
Zellij sessions use other registries and sockets and are not touched. A failed
run writes its raw, test-only PTY transcript to a named `/tmp` file on the Mac;
`--artifacts <path>` keeps the transcript for a successful run too.

## What it proves

The test uses `TERM=xterm-ghostty` and `COLORTERM=truecolor` across the actual
macOS OpenSSH PTY boundary. It verifies:

- startup and workspace attachment;
- transition from the borderless Manage surfaces to Blackpepper's owned
  `bp  blackpepper` terminal status row (there is no literal `WORK` badge);
- terminal-mode setup, including SGR mouse input;
- ordinary shell input and the inherited terminal identity;
- outer and embedded PTY resize propagation;
- mouse-wheel scroll plus keyboard search using Zellij 0.44.3;
- a bounded OSC 52 write returned to the outer macOS terminal, together with
  Blackpepper's brief clipboard-handoff notice;
- an actionable unknown-command error; and
- clean quit plus restoration of alternate-screen and mouse modes.

## What it does not prove

The harness does not claim pixel-level Ghostty rendering, a successful macOS
clipboard write, click targeting, or interactive password/FIDO prompts. Those
require an operator-visible Ghostty window or a purpose-built UI driver. It
also runs the installed Linux `bp-dev`, matching the common “Ghostty SSH'd into
the devbox” workflow; native macOS-client build/runtime coverage remains the
macOS CI job's responsibility.

This distinction matters: receiving Blackpepper's normalized OSC 52 sequence
proves the clipboard request reaches the outer terminal, but only Ghostty and
macOS can decide whether to accept that request into the system clipboard.

The live harness also proves only a clean remote quit. The hermetic
`scripts/test-ssh-terminal-guard.sh` test uses a new caller PTY and a fake SSH
data channel to prove that abrupt channel loss returns status `255`, restores
the exact termios snapshot, emits the DEC reset after the failed session, and
leaves the caller shell usable. It also exercises every handled signal and the
missing-system-SSH and no-controlling-TTY failure paths; it never opens a real
SSH connection or an existing Ghostty window.
