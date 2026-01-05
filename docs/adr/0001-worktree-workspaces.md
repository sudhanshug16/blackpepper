# 0001 - Workspaces via git worktree

Date: 2026-01-04

## Status

Accepted

## Context

Blackpepper needs isolated workspaces for parallel agent tabs without cloning
full repositories for each task.

## Decision

Use `git worktree` to create workspace copies under `./.blackpepper/workspaces/<animal>`.
Branches start as animal names and are renamed after the first task is defined.

## Consequences

- Workspace lifecycle is explicit and manual.
- We must avoid nesting worktrees and keep them out of `node_modules/`.
- Commands like `:create` and `:destroy` must handle worktree cleanup safely.
