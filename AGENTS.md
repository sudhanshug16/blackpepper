# Repository Guidelines

## Overview
Blackpepper is a terminal orchestrator for TUI coding agents. It embeds provider
TUIs (Codex, Claude Code, OpenCode, and future agents) as iframe-like panes,
while Blackpepper owns the sidebar, shortcuts, and workspace controls.

## Project Structure & Module Organization
- `src/`: runtime code. Expected modules include `src/orchestrator/`,
  `src/workspaces/`, `src/providers/`, and `src/cli/` as they are added.
- `docs/`: design notes, ADRs, and usage examples.
- `package.json`, `tsconfig.json`: tooling and TypeScript configuration.

## Workspace & Task Model
- Workspaces are created with `git worktree` under `./workspaces/<animal>`.
  Keep worktrees out of `node_modules/` and do not nest worktrees within
  worktrees.
- Workspace branches start with animal names (e.g., `otter`, `lynx`) and are
  renamed after the first task is defined.
- A workspace can run multiple agent tabs in parallel; each tab may target a
  different provider.
- Workspace lifecycle is manual via command mode: `:create`, `:destroy`,
  `:create-pr`, `:open-pr`, `:merge-pr`.

## Build, Test, and Development Commands
- `bun install`: install dependencies.
- `bun dev`: run locally in watch mode (see `package.json` for the entry point).
- Planned: `bun test`, `bun run build`, `bun run lint`, `bun run format`.

## Coding Style & Naming Conventions
- TypeScript with ESM (`type: "module"`).
- Formatting: Prettier. Linting: ESLint.
- Indentation: 2 spaces; keep trailing commas where already used.
- Naming: PascalCase for types/classes, camelCase for functions/variables, and
  lowercase/kebab-case for filenames. Provider adapters live under
  `src/providers/`.

## Configuration & Secrets
- Config resolution order: workspace-local
  `.config/blackpepper/pepper.toml`, then user-level
  `~/.config/blackpepper/pepper.toml`.
- Validate config on startup and fail with actionable errors.
- Never commit configs or secrets; redact any sensitive values in logs.

## Testing Guidelines
- Use `bun test` once configured.
- Place tests under `tests/` or `src/__tests__/` using `*.test.ts`.
- Prioritize coverage for worktree creation, tab management, provider launch,
  and config merge rules.

## Commit & Pull Request Guidelines
- Use Conventional Commits (`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`,
  `test:`).
- PRs should include a short summary, run instructions, linked issues, and CLI
  output samples for UX changes. Add ADRs and `docs/` examples for new commands.

## AI Contribution Notes
- Record validation steps and assumptions in PRs or notes.
- Prefer dry-run paths for destructive git actions when available.
