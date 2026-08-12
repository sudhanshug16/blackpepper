# macOS SSH PTY acceptance

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
