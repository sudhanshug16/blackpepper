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

## Project Structure & Module Organization

- `crates/blackpepper/src/`: Runtime code (TUI, workspaces, config, PTY).
- `docs/`: ADRs and examples.

## Workspace & Task Model

- Workspaces are created with `git worktree` under `./workspaces/<animal>`; avoid nesting under `node_modules/`.
- Workspace branches start with animal names (e.g., `otter`, `lynx`) and are renamed after the first task is defined.
- A workspace can run multiple agent tabs; each tab may target a provider.
- Workspace lifecycle is manual via `:create`, `:destroy`, `:create-pr`, `:open-pr`, `:merge-pr`.

## CLI & Command Mode

- Entry point is `pepper` (no subcommands).
- Command mode uses `:` prefixes for workspace and PR actions.

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

## Configuration & Secrets

- Config resolution order: workspace-local `.config/blackpepper/pepper.toml`, then user-level `~/.config/blackpepper/pepper.toml`.
- Validate config on startup and fail with actionable errors.
- Never commit configs or secrets; redact any sensitive values in logs.

## Logging & State

- Follow XDG locations; store logs under `~/.local/state/blackpepper/`.

## Testing Guidelines

- Place tests under `crates/blackpepper/tests/` or module `mod tests` blocks.
- Prioritize coverage for worktree creation, tab management, provider launch, and config merge rules.

## Commit & Pull Request Guidelines

- Use Conventional Commits (`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`).
- PRs should include a summary, run instructions, linked issues, and UX samples. Add ADRs and `docs/` examples for new commands.

## AI Contribution Notes

- Record validation steps and assumptions in PRs or notes.
- Avoid one-off CLI/TUI behavior in callers. Use shared command logic (e.g., `CommandSource`) to branch output instead of special-casing in `main.rs` or the UI.
