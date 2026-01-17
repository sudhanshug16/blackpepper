# Conductor Parity Roadmap (Blackpepper)

## Scope
List of Conductor features to implement in Blackpepper. No timelines included.

## Workspace
- [x] Add a `:rename` command to rename the workspace and its branch.
- [x] Add workspace setup scripts for bootstrapping non-git files and local setup.
- Surface setup script completion status (success/failure) in the workspace UI.

## Core UX (Tmux)
- [x] Allow configuration to define tmux tabs created when a workspace starts.
- [x] Always run setup scripts in the first tmux tab.
- Show success status when setup scripts complete.

## Testing & Ports
- [x] Allocate a block of 10 ports per workspace.
- [x] Store the block in `WORKSPACE_PORT_0` through `WORKSPACE_PORT_9`.
- [x] Auto-load the port environment in all tmux tabs, including user-created tabs.

## Workspace Creation
- Create workspaces from existing PRs (select PR, checkout branch, and initialize workspace).

## Config
- No changes planned (current configuration layering is sufficient).
