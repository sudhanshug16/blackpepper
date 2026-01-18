# Conductor Parity Roadmap (Blackpepper)

## Scope
List of Conductor features to implement in Blackpepper. No timelines included.

## Workspace
- [x] Add a `:workspace rename` command to rename the workspace and its branch.
- [x] Add workspace setup scripts for bootstrapping non-git files and local setup.

## Core UX (Tmux)
- [x] Allow configuration to define tmux tabs created when a workspace starts.
- [x] Always run setup scripts in the first tmux tab.

## Testing & Ports
- [x] Allocate a block of 10 ports per workspace.
- [x] Store the block in `WORKSPACE_PORT_0` through `WORKSPACE_PORT_9`.
- [x] Auto-load the port environment in all tmux tabs, including user-created tabs.

## Workspace Creation
- [x] Create workspaces from existing PRs (`:workspace from-pr`) or branches (`:workspace from-branch`).

## Config
- No changes planned (current configuration layering is sufficient).
