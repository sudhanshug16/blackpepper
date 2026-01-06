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

## Configuration

Config resolution order:

1. `./.config/blackpepper/config.toml`
2. `~/.config/blackpepper/config.toml`

Example:

```toml
[keymap]
toggle_mode = "ctrl+g"
switch_workspace = "ctrl+p"

[tmux]
command = "tmux"
args = ["-f", "/path/to/tmux.conf"]

[workspace]
root = ".blackpepper/workspaces"

[agent]
provider = "codex"
command = "custom agent command {{PROMPT}}"

[upstream]
provider = "github"

```

If `[tmux]` is omitted, Blackpepper uses `tmux` from `PATH`.
If `[agent].provider` is set, `:pr create` uses the built-in agent templates; set
`[agent].command` to override the command (optional `{{PROMPT}}` placeholder).
`[upstream].provider` selects the PR backend (default `github` via the `gh` CLI).

State:

- Active workspaces are tracked in `~/.config/blackpepper/state.toml` under `[active_workspaces]`.
- Each entry maps a project root (git common dir) to the last active worktree path.

Example:

```toml
[active_workspaces]
"/path/to/blackpepper" = "/path/to/blackpepper/.blackpepper/workspaces/otter"
```

## Workspaces

Workspaces are created via `git worktree` under `./.blackpepper/workspaces/<animal>`
by default and each workspace attaches to its own tmux session (use tmux windows/panes
for parallel shells). Override the root with `[workspace].root` in `config.toml`.

Run `bp init` (or `:init` inside the TUI) to add gitignore entries and
create an empty project config at `./.config/blackpepper/config.toml`.

Selecting a workspace starts an embedded tmux client in that worktree. Customize
the tmux command with `[tmux]`.

## Modes

- Work mode forwards input to the embedded terminal.
- Manage mode enables global controls (`Ctrl+G` to toggle).
- Use `:` in Manage mode to open the command line (hidden by default).
- Use `Ctrl+P` to open the workspace switcher overlay.
- Use `Esc` to close the command line or return to Work mode.

## Docs

See `docs/` for ADRs and CLI examples, including command mode flows.
