# Parallel Workspace Manager for Coding Agents

Run multiple AI coding agents in parallel on a single Git project. Create
isolated workspaces, see what each agent is doing, then review and merge the
results on your terms.

## Status

We are migrating the stack. The current crate includes a basic TUI with
command mode, worktree management, and a wired PTY/ANSI terminal renderer.

## Quickstart

```bash
cargo run -p blackpepper
```

If installed, run `bp` to launch the TUI.

## Updating

Re-run the installer to fetch the latest release asset:

```bash
curl -fsSL https://raw.githubusercontent.com/sudhanshug16/blackpepper/main/docs/install.sh | bash
```

## Tooling

- `cargo build -p blackpepper`: build the binary.
- `cargo test -p blackpepper`: run tests.
- `cargo fmt`: format sources.
- `cargo clippy --workspace -- -D warnings`: lint.

Terminal stack:

- PTY: `portable-pty`
- ANSI parsing: `vt100`
- Input decoding: `termwiz`

## Configuration

Config resolution order:

1. `./.config/blackpepper/config.toml`
2. `~/.config/blackpepper/config.toml`

Example:

```toml
[keymap]
toggle_mode = "ctrl+]"
switch_workspace = "ctrl+p"

[tmux]
command = "tmux"
args = ["-f", "/path/to/tmux.conf"]

[tmux.tabs.work]
command = "npm run dev"

[tmux.tabs.server]

[workspace]
root = ".blackpepper/workspaces"

[workspace.setup]
scripts = ["./scripts/setup.sh", "make bootstrap"]

[agent]
provider = "codex"
command = "custom agent command {{PROMPT}}"

[upstream]
provider = "github"

[ui]
background = "#333333"
foreground = "#ffffff"

```

Invalid key chords are treated as unbound.

If `[tmux]` is omitted, Blackpepper uses `tmux` from `PATH`.
If `[agent].provider` is set, `:pr create` uses the built-in agent templates; set
`[agent].command` to override the command (optional `{{PROMPT}}` placeholder).
`[upstream].provider` selects the PR backend (default `github` via the `gh` CLI).
Workspace setup scripts in `[workspace.setup].scripts` run in the first tmux tab
(`setup`) when a workspace session starts and can be re-run with `:workspace setup`.
`[tmux.tabs.<name>].command` configures additional tabs (and optional first
commands); if no tabs are defined, Blackpepper opens a single default `work` tab
(the `setup` tab only appears when scripts exist).

State:

- Active workspaces are tracked in `~/.config/blackpepper/state.toml` under `[active_workspaces]`.
- Each entry maps a project root (git common dir) to the last active worktree path.
- Workspace port blocks live under `[workspace_ports]`; each workspace gets 10 ports
  exported as `WORKSPACE_PORT_0` through `WORKSPACE_PORT_9` in tmux sessions.

Example:

```toml
[active_workspaces]
"/path/to/blackpepper" = "/path/to/blackpepper/.blackpepper/workspaces/otter"

[workspace_ports]
"/path/to/blackpepper/.blackpepper/workspaces/otter" = 30000
```

## Workspaces

Workspaces are created via `git worktree` under `./.blackpepper/workspaces/<animal>`
by default and each workspace attaches to its own tmux session (use tmux windows/panes
for parallel shells). Override the root with `[workspace].root` in `config.toml`.

Run `bp init` (or `:init` inside the TUI) to add gitignore entries and
create an empty project config at `./.config/blackpepper/config.toml`.

Selecting a workspace starts an embedded tmux client in that worktree. Blackpepper
enables tmux `extended-keys` for these sessions so modified keys can be preserved
when your terminal supports them. If `COLORTERM` advertises truecolor, Blackpepper
also appends a tmux `terminal-overrides` entry for the current `TERM` so tmux emits
RGB colors. To support TUIs that query default colors (OSC 10/11), Blackpepper
responds with the configured `[ui]` background/foreground. Customize the tmux
command with `[tmux]`.

## Tmux Clipboard and Notifications

Blackpepper is a transparent layer for tmux in work mode. Clipboard integration
relies on tmux copy-mode emitting OSC 52, so ensure `set-clipboard on` is enabled
in your tmux config. Blackpepper enables `allow-passthrough` for its sessions so
OSC notifications reach your host terminal.

Test a notification from a tmux pane:

```sh
printf '\033Ptmux;\033\033]9;Build done\007\033\\'
```

Cross-platform notes:

- OSC 52 clipboard works when a GUI clipboard is available (macOS/Windows/Linux with X11 or Wayland); headless sessions ignore clipboard requests.
- Notifications use `osascript` on macOS and `notify-rust` on Linux/Windows; on Linux this requires a running D-Bus notification daemon.

## Modes

- Work mode is raw input passthrough to tmux; only the toggle sequences are intercepted.
- Toggle mode uses the configured chord (default: `Ctrl+]`).
- Manage mode enables global controls (default toggle: `Ctrl+]`).
- Use `:` in Manage mode to open the command line (hidden by default).
- Use `Ctrl+P` to open the workspace switcher overlay.
- Use `Esc` to close the command line or return to Work mode.

To capture raw input for debugging, set `BLACKPEPPER_DEBUG_INPUT=1` to log to
`/tmp/blackpepper-input.log`.

Note: Some terminals send the same byte for Enter and Shift+Enter. In raw
passthrough mode, tmux cannot distinguish them, so Shift+Enter behaves like
Enter unless your terminal emits a distinct sequence. See
https://github.com/openai/codex/discussions/3024 for background.

## Docs

See `docs/` for ADRs and CLI examples, including command mode flows.

Blackpepper is inspired by Conductor (https://docs.conductor.build/).
