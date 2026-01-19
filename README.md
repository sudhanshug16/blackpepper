# Blackpepper

A TUI orchestrator for AI coding agents built around tmux. Each workspace is
a git worktree with its own tmux session—you get full tmux capabilities
(windows, panes, copy-mode, scrollback) while Blackpepper handles workspace
isolation, port allocation, and agent lifecycle.

**How it works:**

- Each workspace runs in a dedicated tmux session with configurable tabs
  (agent, server, git, or custom commands)
- Blackpepper embeds tmux with raw input passthrough—it's a transparent layer,
  not a terminal emulator replacement
- Workspaces are isolated git worktrees so agents work on separate branches
  without conflicts
- Each workspace gets dedicated ports (`WORKSPACE_PORT_0`–`WORKSPACE_PORT_9`)
  so dev servers don't collide
- PR creation integrates with GitHub via `gh` CLI

**Supported agents:** Codex, Claude Code, OpenCode (configurable via `[agent]`)

## Status

Core features are stable and working: workspace lifecycle (create, destroy,
rename, from-branch, from-pr), tmux session management, port allocation,
config layering, and PR creation. The `:pr open` and `:pr merge` commands
are not yet implemented.

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

Config resolution order (later sources override earlier):

1. `~/.config/blackpepper/config.toml` — User global config
2. `./.config/blackpepper/config.toml` — Project config (committed)
3. `./.config/blackpepper/config.local.toml` — User-project config (gitignored)

The local config is useful for personal preferences that shouldn't be committed,
like opening nvim by default:

```toml
# .config/blackpepper/config.local.toml
[tmux.tabs.nvim]
command = "nvim"
```

Example project config:

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

[workspace.env]
# Custom env vars injected into all tmux tabs
API_URL = "http://localhost:$WORKSPACE_PORT_0"

[git]
remote = "origin"

[agent]
provider = "codex"
command = "custom agent command {{PROMPT}}"

[upstream]
provider = "github"

[ui]
background = "#333333"
foreground = "#ffffff"
```

### Config Reference

- **`[keymap]`** — Key bindings. Invalid chords are treated as unbound.
- **`[tmux]`** — Tmux command and args. Defaults to `tmux` from `PATH`.
- **`[tmux.tabs.<name>]`** — Additional tabs with optional startup commands. If no tabs are defined, Blackpepper opens a single `work` tab.
- **`[workspace]`** — Workspace root directory (default: `.blackpepper/workspaces`).
- **`[workspace.setup]`** — Setup scripts run in the first tmux tab when a workspace starts. Re-run with `:workspace setup`.
- **`[workspace.env]`** — Custom env vars injected into all tmux tabs. Supports `$VAR` and `${VAR}` expansion.
- **`[git]`** — Git settings. `remote` defaults to `origin` (used by `:workspace from-branch` and `:workspace from-pr`).
- **`[agent]`** — AI agent provider and command. `{{PROMPT}}` placeholder is replaced with the generated prompt.
- **`[upstream]`** — PR backend (default: `github` via `gh` CLI).
- **`[ui]`** — Background/foreground colors for the TUI.

## Workspace Ports

Each workspace gets 10 dedicated ports exported as `WORKSPACE_PORT_0` through
`WORKSPACE_PORT_9` in all tmux tabs. Ports are allocated from range 30000-39999
and persist across sessions.

Use these ports directly in your commands:

```bash
# Start Rails on the workspace's first port
rails server -p $WORKSPACE_PORT_0

# Start a frontend dev server on the second port
npm run dev -- --port $WORKSPACE_PORT_1
```

Or define computed env vars in your config:

```toml
[workspace.env]
RAILS_ORIGIN = "http://localhost:$WORKSPACE_PORT_0"
API_URL = "http://localhost:$WORKSPACE_PORT_1/api"
```

These are expanded and injected into all tmux tabs, so you can reference them:

```bash
# In any tmux tab
echo $RAILS_ORIGIN  # => http://localhost:30000
curl $API_URL/health
```

## Workspace Setup Example

A typical monorepo setup with Rails backend and JS frontend:

```toml
# .config/blackpepper/config.toml

[workspace.setup]
scripts = [
    "mise trust",                          # Trust mise config
    "yarn install",                        # Install JS dependencies
    "cd apps/api && bundle install",       # Install Ruby gems
]

[workspace.env]
API_ORIGIN = "http://localhost:$WORKSPACE_PORT_0"
WEB_ORIGIN = "http://localhost:$WORKSPACE_PORT_1"
EXPO_ORIGIN = "http://localhost:$WORKSPACE_PORT_2"

[tmux.tabs.api]
command = "cd apps/api && rails s -p $WORKSPACE_PORT_0"

[tmux.tabs.web]
command = "cd apps/web && npm run dev -- --port $WORKSPACE_PORT_1"

[tmux.tabs.mobile]
command = "cd apps/mobile && npx expo start --port $WORKSPACE_PORT_2"

[tmux.tabs.work]
# Empty tab for general work
```

The `$BLACKPEPPER_REPO_ROOT` env var points to the git repository root,
useful for referencing files relative to the project:

```toml
[workspace.setup]
scripts = ["ln -sf $BLACKPEPPER_REPO_ROOT/.env .env"]
```

## State

- Active workspaces are tracked in `~/.config/blackpepper/state.toml` under `[active_workspaces]`.
- Each entry maps a project root (git common dir) to the last active worktree path.
- Workspace port blocks live under `[workspace_ports]`; each workspace gets 10 ports.

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

### Creating Workspaces

```bash
# Create a new workspace (auto-generates animal name)
:workspace create

# Create with a specific name
:workspace create my-feature

# Create from an existing remote branch
:workspace from-branch feature/auth

# Create from an existing PR (by number or URL)
:workspace from-pr 123
:workspace from-pr https://github.com/org/repo/pull/123
```

Branch names are normalized to valid workspace names (e.g., `feature/auth` becomes
`feature-auth`). The `from-branch` and `from-pr` commands fetch from the configured
`[git].remote` (default: `origin`).

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
