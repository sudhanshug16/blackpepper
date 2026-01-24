# Blackpepper

```
                _     _  _
|_  |  _  _  | |_) _ |_)|_) _  __
|_) | (_|(_  |<|  (/_|  |  (/_ |
```

A TUI orchestrator for AI coding agents built around tmux. Each workspace is
a git worktree with its own tmux session—you get full tmux capabilities
(windows, panes, copy-mode, scrollback) while Blackpepper handles workspace
isolation, port allocation, and agent lifecycle.

**Supported agents:** Codex, Claude Code, OpenCode (configurable via `[agent]`)

## Status

Core features are stable: workspace lifecycle, tmux session management, port
allocation, config layering, and PR creation. The `:pr open` and `:pr merge`
commands are not yet implemented.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/sudhanshug16/blackpepper/main/docs/install.sh | bash
```

Or build from source:

```bash
cargo install --path crates/blackpepper
```

Run `bp` to launch. Re-run the installer to update.

## How To

### Modes

Blackpepper has two modes:

| Mode       | What it does                                                             |
| ---------- | ------------------------------------------------------------------------ |
| **Work**   | Raw input passthrough to tmux. All keys go to tmux except toggle chords. |
| **Manage** | Global controls. Navigate workspaces, run commands, quit.                |

### Navigation

| Key       | Action                         | Works in     |
| --------- | ------------------------------ | ------------ |
| `Ctrl+]`  | Toggle between Work ↔ Manage  | Both         |
| `Ctrl+\\` | Open workspace switcher        | Both         |
| `Ctrl+n`  | Cycle workspaces               | Both         |
| `:`       | Open command line              | Manage       |
| `q`       | Quit                           | Manage       |
| `Esc`     | Close overlay / return to Work | Manage       |
| `j` / `k` | Navigate lists                 | Overlays     |
| `Enter`   | Select / confirm               | Overlays     |
| `Tab`     | Autocomplete                   | Command line |

### Common Tasks

```bash
# Initialize project (adds gitignore, creates config template)
:init

# Create a new workspace
:workspace create
:workspace create my-feature

# Create from existing local/remote branch or PR
:workspace from-branch feature/auth
:workspace from-pr 123

# Switch workspaces
Ctrl+\\                    # open switcher overlay
Ctrl+n                    # cycle to next workspace
:workspace list           # open switcher overlay
:workspace switch otter   # switch directly

# Rename workspace and branch
:workspace rename new-name

# Run setup scripts again
:workspace setup

# Show allocated ports
:ports

# Create a PR
:pr create

# Destroy workspace
:workspace destroy
:workspace destroy otter
```

## Commands

All commands work in the TUI (`:command`) and CLI (`bp command`).

| Command                          | Description                                       |
| -------------------------------- | ------------------------------------------------- |
| `init`                           | Initialize project config and gitignore           |
| `workspace create [name]`        | Create workspace (auto-generates name if omitted) |
| `workspace destroy [name]`       | Destroy workspace worktree                        |
| `workspace rename <name>`        | Rename workspace and branch                       |
| `workspace switch <name>`        | Switch to workspace                               |
| `workspace from-branch <branch>` | Create workspace from local or remote branch      |
| `workspace from-pr <number>`     | Create workspace from PR                          |
| `workspace setup`                | Re-run setup scripts                              |
| `workspace list`                 | List workspaces                                   |
| `ports`                          | Show allocated ports                              |
| `pr create`                      | Create PR from current workspace                  |
| `update`                         | Update to latest release                          |
| `refresh`                        | Refresh repo status                               |
| `quit` / `q`                     | Exit (detaches tmux sessions)                     |

## Configuration

Config resolution order (later overrides earlier):

1. `~/.config/blackpepper/config.toml` — User global
2. `./.config/blackpepper/config.toml` — Project (committed)
3. `./.config/blackpepper/config.local.toml` — Local (gitignored)

### Minimal Config

```toml
[keymap]
toggle_mode = "ctrl+]"
switch_workspace = "ctrl+n"
workspace_overlay = "ctrl+\\"

[agent]
provider = "claude"  # or "codex", "opencode"

[tmux.tabs.work]
# empty tab for general work
```

### Full Example

```toml
[keymap]
toggle_mode = "ctrl+]"
switch_workspace = "ctrl+n"
workspace_overlay = "ctrl+\\"

[tmux]
command = "tmux"
args = ["-f", "/path/to/tmux.conf"]

[tmux.tabs.api]
command = "cd apps/api && rails s -p $WORKSPACE_PORT_0"

[tmux.tabs.web]
command = "cd apps/web && npm run dev -- --port $WORKSPACE_PORT_1"

[tmux.tabs.work]

[workspace]
root = ".blackpepper/workspaces"

[workspace.setup]
scripts = ["mise trust", "yarn install"]

[workspace.env]
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

| Section              | Description                                                           |
| -------------------- | --------------------------------------------------------------------- |
| `[keymap]`           | Key bindings (`toggle_mode`, `switch_workspace`, `workspace_overlay`) |
| `[tmux]`             | Tmux command and args                                                 |
| `[tmux.tabs.<name>]` | Tabs with optional startup commands                                   |
| `[workspace]`        | Workspace root directory                                              |
| `[workspace.setup]`  | Setup scripts run on workspace start                                  |
| `[workspace.env]`    | Env vars injected into all tmux tabs                                  |
| `[git]`              | Git remote (default: `origin`)                                        |
| `[agent]`            | Provider and custom command template                                  |
| `[upstream]`         | PR backend (default: `github`)                                        |
| `[ui]`               | TUI colors                                                            |

Note: The built-in `git` tab defaults to `gitui` (install it first). Lazygit is
not a good fit for Blackpepper worktrees, so configure a different git command if
you prefer another UI.

## Workspaces

Workspaces are git worktrees under `.blackpepper/workspaces/<name>`. Each
workspace gets its own tmux session. Run `:init` to set up gitignore entries.

### Workspace Ports

Each workspace gets 10 ports (`WORKSPACE_PORT_0`–`WORKSPACE_PORT_9`) allocated
from range 30000-39999. These persist across sessions.

```bash
rails server -p $WORKSPACE_PORT_0
npm run dev -- --port $WORKSPACE_PORT_1
```

Reference in config:

```toml
[workspace.env]
API_URL = "http://localhost:$WORKSPACE_PORT_0"
```

### Setup Scripts

Scripts run in the first tmux tab when a workspace starts:

```toml
[workspace.setup]
scripts = [
    "mise trust",
    "yarn install",
    "cd apps/api && bundle install",
]
```

Re-run with `:workspace setup`. The `$BLACKPEPPER_REPO_ROOT` env var points to
the git repository root.

## Tmux Integration

Blackpepper is a transparent layer for tmux. Use tmux for:

- Windows and panes (`Ctrl+b` commands)
- Copy mode and scrollback
- Session management

### Clipboard

Ensure `set-clipboard on` in your tmux config. Blackpepper passes OSC 52
sequences to the host terminal.

### Notifications

Blackpepper enables `allow-passthrough` so notifications reach your terminal:

```sh
printf '\033Ptmux;\033\033]9;Build done\007\033\\'
```

### Platform Notes

- Clipboard requires GUI (macOS/Windows/Linux with X11 or Wayland)
- Notifications use `osascript` (macOS) or D-Bus (Linux)

## State

Active workspace and ports are tracked in `~/.config/blackpepper/state.toml`:

```toml
[active_workspaces]
"/path/to/project" = "/path/to/project/.blackpepper/workspaces/otter"

[workspace_ports]
"/path/to/project/.blackpepper/workspaces/otter" = 30000
```

## Development

```bash
cargo build -p blackpepper    # build
cargo test -p blackpepper     # test
cargo fmt                     # format
cargo clippy --workspace -- -D warnings  # lint
```

### Debug Input

Set `BLACKPEPPER_DEBUG_INPUT=1` to log raw input to `/tmp/blackpepper-input.log`.

## Docs

See `docs/` for ADRs and examples. Inspired by [Conductor](https://docs.conductor.build/).
