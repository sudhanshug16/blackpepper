# Blackpepper

Blackpepper is a remote-first terminal workspace for coding agents. It gives
local folders and folders on Linux SSH hosts the same UI, while Zellij keeps
shells and agent tabs alive on the machine where the work runs.

V1 is a standalone macOS/Linux client. It does not embed a Zellij plugin and
does not depend on tmux, Ghostty, or a remote-desktop protocol.

## Current status

The V1 client is usable from source for local work. Release bundles add the
native `bp-host` helper plus static Linux x86_64 and arm64 helpers needed to
bootstrap arbitrary supported SSH hosts. The core flows are wired: host
registration, interactive SSH login, remote registry discovery, Zellij attach,
Worktrunk mutations with approval, listener discovery, loopback forwarding,
launch-scoped agent adapters, and live-run status recovery after client restart.

The rebuild is not release-complete yet. In particular:

- SSH reconnection is manual. Reconnecting restores registered shells,
  `auto_start` service tabs, and each forward on its original local port; it
  never resumes an agent conversation.
- Live direct transport, first-use host-key, reconnect, parallel-channel,
  fail-closed mux, stdin, cancellation, managed-sidecar, ProxyJump,
  encrypted-key passphrase, and changed-host-key checks pass on Linux.
  Deterministic end-to-end provider/privacy fixtures also pass through the real
  helper, Zellij, SQLite, and TUI. Password, FIDO/security-key, macOS, and
  authenticated-provider acceptance still need the full platform matrix.

See [the V1 roadmap](docs/roadmap.md) for the remaining acceptance work.

## Runtime model

- A **workspace** is one absolute folder on one host. Starting Blackpepper
  registers the current local folder automatically.
- The sidebar is **Host → Repository → Workspace**. Git worktrees with the same
  common directory or normalized primary remote are grouped automatically.
  `:workspace ungroup` persists an exception.
- Every workspace gets a UUID-named Zellij session. Detaching or quitting
  Blackpepper leaves it running; `:workspace terminate` ends the session but
  keeps the folder.
- Agent spawning stays in the selected folder. Creating a worktree is a
  separate, explicit action delegated to Worktrunk.
- SSH carries a normal `zellij attach` PTY byte stream. It is incremental
  terminal output, not video or screenshots.

## Platforms and dependencies

| Role | Supported |
| --- | --- |
| Client and local workspace | macOS or Linux, x86_64 or arm64 |
| SSH workspace host | Linux, x86_64 or arm64 |
| Session runtime | Zellij **0.44.3** |
| Worktree runtime | Worktrunk **0.72.0** |

Blackpepper uses an exact system Zellij or Worktrunk version when available.
Otherwise it downloads the pinned release on the client, verifies the embedded
SHA-256, and installs a versioned sidecar without changing `PATH` or shell
files.

The client needs system OpenSSH. Workspace hosts need a POSIX shell, Git,
`install`, and `sha256sum`; port discovery also needs `ss` on Linux or `lsof`
on macOS. Install and authenticate `codex`, `claude`, or `opencode` on the host
where that agent will run.

## Install

Release archives are checksum-verified before extraction and install as one
versioned bundle, so `bp`, its matching native helper, and both remote Linux
helpers cannot be silently mixed across versions:

```bash
curl -fsSL https://raw.githubusercontent.com/sudhanshug16/blackpepper/main/docs/install.sh | bash
```

For day-to-day development, install the current debug build beside production:

```bash
./scripts/setup.sh
bp-dev
```

This installs the global command `bp-dev` in `$DEV_INSTALL_DIR` (default
`~/.local/bin`) and keeps its exact matching `bp-host` in a private,
build-identified bundle. It never replaces production `bp` or `bp-host`. A
source-built dev install does not contain cross-compiled Linux helpers; use a
release archive for supported macOS-to-Linux or cross-architecture SSH
bootstrap. You can keep one production `bp` client open while rebuilding,
installing, and running one `bp-dev` client. They share the stable
host/workspace/session registry and lifecycle locks, so both channels see the
same Zellij resources and same-workspace mutations serialize. Agent-event
stores are channel-specific. Another process in the same channel reports its
owner PID and exits.

The development build ID is deterministic for the exact source tree, so
reinstalling unchanged code reuses its verified bundle.
`./scripts/setup.sh --list-builds` shows retained builds;
`./scripts/setup.sh --prune <build-id>` removes one explicitly selected
inactive build and refuses the current or a visibly running build. Pruning
warns because a dormant provider hook can still contain that exact helper
path. Installed copies have debug sections stripped to save space; the Cargo
target keeps its full debug artifacts.

For browser-driven TUI inspection, `scripts/web-dev.sh` runs the exact
development bundle inside a one-client, loopback-only ttyd/xterm.js PTY. It
prints a per-launch URL with independent 128-bit authentication and path
secrets for browser controls, and does not change the production install.
Close an existing `bp-dev` client first; production `bp` may remain open.
Validated OSC 52 writes reach the browser through a local
bridge; when browser policy requires a user gesture it shows a visible
**Copy** button. See the [browser development guide](docs/web-dev.md).

For a production-named source install, Cargo can still install both release
binaries together:

```bash
cargo install --path crates/blackpepper
```

Run the production client with no subcommand:

```bash
bp
```

`bp --help` and `bp --version` are supported. V1 has no command-line
subcommands; actions are entered inside the TUI.

## First use

### Local workspace

```bash
cd /path/to/project
bp
```

The current folder is registered. Select it and press `Enter` to create or
attach its Zellij session.

### SSH workspace

Use an existing literal OpenSSH alias so `Include`, `Match`, `ProxyJump`, the
SSH agent, and platform keychain behavior remain OpenSSH-owned:

```text
:host add lab homelab
:host connect lab
:workspace add /srv/projects/example
```

Host-key, password, passphrase, and security-key prompts appear in the client.
`:host import` only previews literal positive aliases from `~/.ssh/config`; add
one explicitly after reviewing it. Private keys are never copied or stored.

### Agents, worktrees, and ports

```text
:agent spawn codex
:worktree create feature/auth --base main
:approve
:ports
:forward 3000
```

Every Worktrunk mutation first displays the exact mutation and every currently
unapproved project hook command. `:approve` is bound to that repository,
mutation argv, and hook plan; any change requires a new review. When Worktrunk
has unapproved project commands, `:approve` also confirms Worktrunk's persistent
approval prompt, verifies the saved plan, and only then runs the mutation.
Press `Esc` to dismiss a review without running or approving anything.
Blackpepper never adds force, clobber, reap, hook-skipping, or force-delete
options.

Removal is host-owned rather than a pair of client-side calls. The helper
checks the stable workspace ID, expected path, registered repository identity,
and actual Git common directory, then records a durable removal intent before
running `wt remove`. A successful command atomically removes the shared
workspace record. If the helper or SSH response is lost after dispatch, a
fresh `:worktree list` compares Worktrunk schema-2 output with that intent: a
missing target completes registry cleanup, while a present target clears the
intent. Reconciliation never reruns `wt remove`, and the refreshed host
registry also removes any client-local ghost.

A host-owned workspace lifecycle lease serializes session restoration,
verified attach, service/agent tab creation, termination, and approved removal
across laptops. The removal helper takes the same gate before dispatch, and a
durable pending-removal marker prevents a crashed operation from recreating a
session until `:worktree list` resolves the folder's actual state.

Port forwarding binds only to client loopback. The first forward prefers the
same local port and reports the actual mapping if another port is selected.
In Manage mode, click a listener in the **Ports** panel for the same action as
`:forward <port>`; use `:ports --all-host` to include all host listeners. A
port-only command is accepted only when discovery identifies one exact socket.
When the same port exists on several interfaces, select the row or use
`:forward <address>:<port>` (IPv6 uses `[address]:port`). Duplicate processes
sharing one socket are rejected because TCP forwarding cannot choose a process.
After a manual reconnect, Blackpepper keeps that exact local URL or reports
`Port conflict`; it never silently moves the forward. SSH forwards and local
proxies end with the client or can be removed with
`:forward cancel <port|address:port>`. A listener already on local loopback is
shown as a **direct URL** instead: cancelling removes the shortcut, not the
service that owns the socket.

## Controls

| Input | Action |
| --- | --- |
| `Ctrl+]` | Toggle Work and Manage modes |
| `Ctrl+n` | Select and attach the next workspace |
| `Ctrl+\` | Return to Manage mode and select the next workspace |
| `↑` / `↓` | Move workspace selection in Manage mode |
| `Enter` | Attach the selected workspace |
| `:` | Enter a Blackpepper command |
| `Esc` | Cancel a command, or return to Work mode |
| `q` | Quit from Manage mode; Zellij sessions keep running |

Work mode gives the terminal the full window except for one quiet status row;
the workspace tree and Ports panel stay in Manage mode. Other input goes to
Zellij unchanged. Use native Zellij
keybindings, panes, tabs, scrolling, search, selection, and copy behavior.
Before creating or attaching a session, Blackpepper runs Zellij's read-only
`setup --check`; an invalid effective configuration is shown without editing
it. The attached client forces only `on_force_close=detach`, so dropping the
client cannot turn a user's `on_force_close "quit"` setting into an accidental
workspace termination. Blackpepper installs no Zellij keybindings or focus
shortcuts in V1.

Commands that can wait on SSH, sidecar provisioning, provider health,
Worktrunk hooks, Zellij, or port discovery run in a host-scoped background
worker. The TUI and already attached terminals remain responsive; commands on
the same host are visibly refused until that worker finishes. Press `Esc` in
Manage mode to cancel. If a Worktrunk mutation may already have been
dispatched, cancellation is reported as an unknown outcome and is never
retried automatically.

Multiple clients may attach to one Zellij session. Zellij 0.44.3 steals a
client's focus when its external API creates a tab, even with `focus=false`, so
Blackpepper refuses background service or agent creation while more than one
client is attached. With one client it restores that exact client's previous
tab after creation. When startup services were created before any client
attached, the first attachment starts its terminal reader, revalidates that it
is still the only client under the workspace lifecycle lease, and selects the
initial shell tab before accepting Work-mode input. Native Zellij tab creation
and selection remain available.
The sidebar refreshes each attached workspace's client count every two seconds.
Same-pane input, scroll, search, selection, and the minimum terminal size
remain shared Zellij state.

## Commands

| Command | Result |
| --- | --- |
| `:host add <name> <ssh-alias>` | Register an SSH destination |
| `:host import` | Preview literal aliases from `~/.ssh/config` |
| `:host connect <name>` | Open the interactive SSH control connection |
| `:host disconnect <name>` | Disconnect; remote Zellij sessions remain |
| `:workspace add <path>` | Register an existing local or remote folder |
| `:workspace switch <name-or-id>` | Select and attach a workspace |
| `:workspace ungroup` | Persistently exclude it from repository grouping |
| `:workspace terminate` | End its Zellij session; keep its folder |
| `:worktree list` | List branches and worktrees using Worktrunk schema 2 |
| `:worktree create <branch> [--base <ref>]` | Preview creation of a worktree |
| `:worktree open <branch-or-PR-or-URL>` | Preview opening a Worktrunk target |
| `:worktree remove` | Preview removal of the selected worktree |
| `:approve` | Approve the reviewed Worktrunk plan and run its mutation |
| `:agent spawn <codex\|claude\|opencode>` | Start an integrated background tab |
| `:service start <name>` | Start a configured service tab |
| `:ports [--all-host]` | Discover workspace or all-host listeners |
| `:forward <port\|address:port>` | Forward one exact discovered listener to client loopback |
| `:forward cancel <port\|address:port>` | Cancel this client's forward |
| `:status explain` | Show redacted agent-status diagnostics |
| `:refresh` | Refresh connected registries, ports, and agent snapshots |
| `:help` | List commands |
| `:quit` / `:q` | Detach and exit |

There are no V1 commands for PR creation, merging, branch renaming, or automatic
worktree creation. Removing a worktree currently requires another registered
worktree from the same repository to use as Worktrunk's surviving cwd.

## Configuration

Configuration is strict TOML. Unknown fields fail startup instead of being
silently ignored. Layers are applied in this order, with later values winning:

1. `$XDG_CONFIG_HOME/blackpepper/config.toml` (normally
   `~/.config/blackpepper/config.toml`)
2. `<launch-folder>/.blackpepper/config.toml`
3. `<launch-folder>/.blackpepper/config.local.toml`

The client's keys, colors, and SSH hosts come from the machine where `bp` was
started. For each local or remote workspace launch, Blackpepper reloads
`[[startup]]` and `[workspace.env]` from that workspace's own host and folder.

Only the user-level file contributes `[hosts.*]` records. Each host accepts
only a `destination` OpenSSH alias; unknown structured fields are rejected.
Put hostname, user, port, jump-host, and key choices in OpenSSH config:

```toml
[keymap]
toggle_mode = "ctrl+]"
switch_workspace = "ctrl+n"
workspace_overlay = "ctrl+\\"

[hosts.lab]
destination = "homelab"
```

Startup services, service environment, and UI colors are active:

```toml
[[startup]]
name = "web"
command = ["npm", "run", "dev"]
cwd = "apps/web"
auto_start = true

[workspace.env]
RUST_LOG = "info"

[ui]
background = "#333333"
foreground = "#ffffff"
```

An `auto_start = true` service starts when Blackpepper creates or recreates the
workspace's Zellij session; `:service start web` starts any named service
explicitly. Commands are argv arrays, not shell strings. Relative service
working directories must stay inside the workspace. `[workspace.env]` is
injected when the default session is created and into each configured service
or provider launch. Blackpepper's launch-scoped integration values take
precedence over conflicting project values.

## Status privacy

Provider integrations are launch-scoped and do not overwrite user provider
configuration. Codex and Claude Code receive a configuration preflight; after
any provider tab starts, Blackpepper requires a health event within five
seconds or returns guided setup instructions. A Codex hook-trust timeout leaves
that tab open so `/hooks` can be reviewed, but deactivates the unhealthy run;
close the tab and retry after trusting the hook. Host-side SQLite stores only
IDs, monotonic sequence numbers, normalized state, source, health, and
timestamps. Hooks discard prompt, response, command, tool, and terminal text.

Pinned blocker rules, conservatively adapted from
[Herdr's agent manifests](https://herdr.dev/docs/agents/), may inspect a Zellij
viewport on the workspace host to add a temporary `needs_input` overlay. They
cannot mark an agent working or done, clear provider state, send input, persist
viewport text, or transmit evidence text to the client. Codex and Claude are
reported as **partial** needs-input coverage; OpenCode can be **full** only
while its managed plugin is healthy. The plugin sends a compact heartbeat every
two seconds. Repeated pulses update one freshness row instead of growing the
event log; after ten seconds without a successful delivery, health becomes
`stale` and the existing host-local watcher may use the blocker overlay. A
later heartbeat restores plugin authority and clears any screen overlay only
when its compact semantic cursor proves that no provider event was lost. A
cursor gap stays **partial** until that launch is restarted.

Each run records its exact session, tab name/ID, pane selector, and Zellij
version. On client restart or SSH reconnect, Blackpepper reconciles that binding
without focusing the session: a live provider gets a new status watcher, a
missing/exited pane becomes `exited`, and no provider command is relaunched.
Ctrl-C persists `unknown` and suppresses a delayed completion until later
authoritative activity proves the run resumed.

## State and migration

Registries use SQLite WAL under `$XDG_STATE_HOME/blackpepper/` (normally
`~/.local/state/blackpepper/`). Client, repository, and session advisory locks
also use its private `run/` subtree so desktop, SSH, and browser launches
coordinate even when they see different `$XDG_RUNTIME_DIR` values. Directories
are mode `0700`; state and lock files are mode `0600`.

One production `bp` client and one development `bp-dev` client may run for a
local OS user. A second process in either channel prints that channel's owner
PID and exits. Both clients share the stable host/workspace/session registry,
repository locks, and session locks; provider event stores remain
channel-specific. This also does not prevent several laptops from attaching
to one remote Zellij session.

Legacy `[tmux]` or `[tmux.*]` configuration is rejected with a migration
message. Old tmux sessions and the old TOML state file are not changed or
imported. See [Remote-first V1 migration](docs/remote-first-v1.md).

## Development

```bash
cargo build -p blackpepper
cargo test --workspace
cargo fmt
cargo clippy --workspace -- -D warnings
scripts/test-dev-installer.sh
scripts/test-web-dev.sh
scripts/test-terminal-transparency-e2e.sh
```

The production binaries are `bp` and `bp-host`. The development installer
publishes only the global `bp-dev` launcher; its helper stays beside the
build-identified private client. Development and production never overwrite
one another. Both helpers are transient: the client starts them for versioned
JSON-lines requests or provider hooks; neither is a daemon.

See [ARCHITECTURE.md](ARCHITECTURE.md) for module boundaries and invariants.

## License

Blackpepper is MIT licensed. Release bundles also include Herdr's Apache-2.0
license and the attribution for the adapted blocker-only manifests.
