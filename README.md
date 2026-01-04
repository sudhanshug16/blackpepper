# blackpepper

blackpepper is a terminal orchestrator for TUI coding agents. It embeds provider
UIs (Codex, Claude Code, OpenCode, and future agents) inside a unified TUI with
shared sidebar controls, shortcuts, and workspace management.

## Quickstart

```bash
bun install
bun dev
```

## Configuration

Config is resolved in this order:

1. `./.config/blackpepper/pepper.toml`
2. `~/.config/blackpepper/pepper.toml`

Example:

```toml
[keymap]
toggle_mode = "ctrl+g"
```

## Workspaces

Workspaces are created via `git worktree` under `./workspaces/<animal>` and can
host multiple agent tabs in parallel.

## Modes

- Normal mode keeps focus in the main TUI.
- Control mode enables the sidebar (`Ctrl+G` to toggle).
- Use `:` in Control mode to open the command line.
- Use `Esc` to close the command line or return to Normal mode.

## Docs

See `docs/` for ADRs and CLI examples, including command mode flows.
