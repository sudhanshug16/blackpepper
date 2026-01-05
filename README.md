# blackpepper

blackpepper is a terminal orchestrator for TUI coding agents. It embeds provider
UIs (Codex, Claude Code, OpenCode, and future agents) inside a unified TUI with
shared controls, shortcuts, and workspace management.

## Status

We are migrating the stack to Rust. The current crate includes a basic TUI with
command mode, worktree management, and a wired PTY/ANSI terminal renderer.

## Quickstart (Rust)

```bash
cargo run -p blackpepper
```

## Tooling

- `cargo build -p blackpepper`: build the binary.
- `cargo test -p blackpepper`: run tests.
- `cargo fmt`: format Rust sources.
- `cargo clippy --workspace -- -D warnings`: lint.

Terminal stack:

- PTY: `portable-pty`
- ANSI parsing: `vt100`

## Configuration

Config resolution order:

1. `./.blackpepper/config.toml`
2. `~/.config/blackpepper/pepper.toml`

Example:

```toml
[keymap]
toggle_mode = "ctrl+g"
switch_workspace = "ctrl+p"

[terminal]
command = "/bin/zsh"
args = ["-l"]

[workspace]
root = ".blackpepper/workspaces"
```

If `[terminal]` is omitted, Blackpepper uses `$SHELL` (or `bash`/`cmd.exe`).

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
by default and can host multiple agent tabs in parallel. Override the root
with `[workspace].root` in `config.toml`.

Run `pepper init` (or `:init` inside the TUI) to add gitignore entries and
create an empty project config at `./.blackpepper/config.toml`.

Selecting a workspace starts an embedded terminal in that worktree. Customize
the shell with `[terminal]`.

## Modes

- Work mode forwards input to the embedded terminal.
- Manage mode enables global controls (`Ctrl+G` to toggle).
- Use `:` in Manage mode to open the command line (hidden by default).
- Use `Ctrl+P` to open the workspace switcher overlay.
- Use `Esc` to close the command line or return to Work mode.

## Docs

See `docs/` for ADRs and CLI examples, including command mode flows.
