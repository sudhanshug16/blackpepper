# Repository Guidelines

## Overview

Blackpepper embeds provider UIs (Codex, Claude Code, OpenCode) inside a Rust TUI
with an embedded shell per workspace.

## Project Structure & Module Organization

- `crates/blackpepper/src/`: Rust runtime code (TUI, workspaces, config, PTY).
- `docs/`: ADRs and examples.

## Workspace & Task Model

- Workspaces are created with `git worktree` under `./workspaces/<animal>`;
  avoid nesting under `node_modules/`.
- Workspace branches start with animal names (e.g., `otter`, `lynx`) and are
  renamed after the first task is defined.
- A workspace can run multiple agent tabs; each tab may target a provider.
- Workspace lifecycle is manual via `:create`, `:destroy`, `:create-pr`,
  `:open-pr`, `:merge-pr`.

## CLI & Command Mode

- Entry point is `pepper` (no subcommands).
- Command mode uses `:` prefixes for workspace and PR actions.

## Build, Test, and Development Commands

- `cargo run -p blackpepper`: run the TUI.
- `cargo build -p blackpepper`: build the binary.
- `cargo test -p blackpepper`: run tests.
- `cargo fmt`: format Rust sources.
- `cargo clippy --workspace -- -D warnings`: lint.

## Coding Style & Naming Conventions

- Rust 2021 edition. Formatting: `rustfmt`. Linting: `clippy`.
- Indentation: 4 spaces (rustfmt defaults).
- Naming: `snake_case` for modules/functions, `CamelCase` for types, `SCREAMING_SNAKE_CASE` for consts.

## Runtime APIs

- Prefer Rust stdlib; use `portable-pty` for PTY access and `vt100` (or equivalent)
  for ANSI parsing when rendering terminals.
- Avoid shelling out unless necessary; centralize git/worktree calls.

## Configuration & Secrets

- Config resolution order: workspace-local
  `.config/blackpepper/pepper.toml`, then user-level
  `~/.config/blackpepper/pepper.toml`.
- Validate config on startup and fail with actionable errors.
- Never commit configs or secrets; redact any sensitive values in logs.

## Logging & State

- Follow XDG locations; store logs under `~/.local/state/blackpepper/`.

## Testing Guidelines

- Place tests under `crates/blackpepper/tests/` or module `mod tests` blocks.
- Prioritize coverage for worktree creation, tab management, provider launch,
  and config merge rules.

## Commit & Pull Request Guidelines

- Use Conventional Commits (`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`,
  `test:`).
- PRs should include a summary, run instructions, linked issues, and UX
  samples. Add ADRs and `docs/` examples for new commands.

## AI Contribution Notes

- Record validation steps and assumptions in PRs or notes.
