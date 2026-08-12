# Repository Guidelines

## Overview

Blackpepper embeds provider UIs (Codex, Claude Code, OpenCode) inside a TUI
with an embedded shell per workspace.

## AI-assisted workflow

- Start with a spec for any non-trivial work: goals, constraints, edge cases, and a testing plan. Ask questions until clear.
- Turn the spec into a small, ordered plan. Implement one step at a time and validate before moving on.
- Pack context before coding: relevant files, commands, invariants, data shapes, and examples. Note what must not change.
- Choose the right model or tool for the task; if stuck, try another model but keep changes consistent.
- Keep a human in the loop: review diffs, run tests, and correct issues.

## Context packing checklist

- Entry points and module boundaries
- Existing patterns to follow
- Constraints (performance, security, compatibility)
- Inputs and outputs, data formats, and edge cases
- Test targets and how to run them

## Quality bar

- Review all generated code and diffs.
- Run targeted tests and share results; if tests are missing, call it out and propose additions.
- For each logical change, add, remove, or update comments so intent stays in sync.
- Record assumptions, risks, and validation steps in PRs or notes.
- Prefer small, surgical changes unless a refactor is explicitly requested.
- If a file grows beyond ~300 lines, split it into smaller modules/files.

## Project Structure & Module Organization

- `crates/blackpepper/src/`: Runtime code (TUI, workspaces, config, PTY).
- `docs/`: ADRs and examples.

## Workspace & Task Model

- A workspace is one registered absolute folder on the local machine or a Linux
  SSH host. Blackpepper does not choose an animal name or directory for it.
- Worktrunk owns Git worktree list/create/open/remove. Every mutation is
  previewed and must be confirmed with the exact-plan `:approve` flow.
- A workspace can run several Zellij tabs for shells, configured services, and
  coding agents. Reconnect restores shells/services, never agent conversations.
- Terminating a workspace ends its Zellij session but keeps the folder;
  worktree removal is a separate, journaled Worktrunk operation.

## CLI & Command Mode

- Production entry point is `bp`; the side-by-side source build is `bp-dev`.
  Both have no CLI subcommands beyond `--help` and `--version`.
- Inside the TUI, `:` commands cover hosts, workspaces, Worktrunk, agents,
  services, ports, status, refresh, help, and quit. V1 has no PR create/merge,
  branch-rename, or automatic-worktree command surface.
- Keep host operations that may wait on SSH, helpers, Zellij, Worktrunk,
  providers, or port probes off the render thread.

## Build, Test, and Development Commands

- `cargo run -p blackpepper`: run the TUI.
- `cargo build -p blackpepper`: build the binary.
- `cargo test -p blackpepper`: run tests.
- `cargo fmt`: format sources.
- `cargo clippy --workspace -- -D warnings`: lint.

## Coding Style & Naming Conventions

- 2021 edition. Formatting: `rustfmt`. Linting: `clippy`.
- Indentation: 4 spaces (rustfmt defaults).
- Naming: `snake_case` for modules/functions, `CamelCase` for types, `SCREAMING_SNAKE_CASE` for consts.

## Runtime APIs

- Prefer stdlib; use `portable-pty` for PTY access and `vt100` (or equivalent) for ANSI parsing when rendering terminals.
- Avoid shelling out unless necessary; centralize git/worktree calls.

## Terminal Transparency Principle

- In work mode, Blackpepper is a transparent layer for Zellij. Do not replace
  Zellij selection, copy, scrollback search, tab, or pane behavior.
- Handle bounded OSC 52 writes so Zellij copy-mode reaches the system or outer
  browser clipboard; never answer clipboard reads or persist clipboard text.

## Configuration & Secrets

- Config resolution order is user
  `$XDG_CONFIG_HOME/blackpepper/config.toml`, then
  `<workspace>/.blackpepper/config.toml`, then the optional ignored
  `<workspace>/.blackpepper/config.local.toml`; later layers win.
- Only the user layer may define SSH hosts. Host entries contain an OpenSSH
  destination alias, not structured hostname/key/jump-host settings.
- Validate config on startup and fail with actionable errors.
- Never commit configs or secrets; redact any sensitive values in logs.

## Logging & State

- Follow XDG locations; store logs under `~/.local/state/blackpepper/`.
- Production `bp` and development `bp-dev` intentionally share the
  host/workspace/session registry and lifecycle locks. Keep that V1 storage
  schema and its persisted JSON backward-compatible with the latest production
  build; put launch-specific provider state in the channel-specific agent-event
  database. An incompatible registry model requires an explicit new storage
  epoch and migration design, not an in-place dev schema bump.

## Testing Guidelines

- Place tests under `crates/blackpepper/tests/` or module `mod tests` blocks.
- Prioritize coverage for worktree creation, tab management, provider launch, and config merge rules.

## Commit & Pull Request Guidelines

- Use Conventional Commits (`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`).
- PRs should include a summary, run instructions, linked issues, and UX samples. Add ADRs and `docs/` examples for new commands.

## AI Contribution Notes

- Record validation steps and assumptions in PRs or notes.
- Avoid one-off CLI/TUI behavior in callers. Use shared command logic (e.g., `CommandSource`) to branch output instead of special-casing in `main.rs` or the UI.
