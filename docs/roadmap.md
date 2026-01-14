# Conductor Parity Roadmap (Blackpepper)

## Scope
List of Conductor features to implement in Blackpepper. No timelines included.

## Workspace
- Add a `:rename` command to rename the workspace and its branch.
- Add workspace setup scripts for bootstrapping non-git files and local setup.
- Surface setup script completion status (success/failure) in the workspace UI.

## Core UX (Tmux)
- Allow configuration to define tmux tabs created when a workspace starts.
- Always run setup scripts in the first tmux tab.
- Show success status when setup scripts complete.

## Testing & Ports
- Allocate a block of 10 ports per workspace.
- Store the first port in an environment variable (name TBD).
- Auto-load the port environment in all tmux tabs, including user-created tabs.

## Workspace Creation
- Create workspaces from existing PRs (select PR, checkout branch, and initialize workspace).

## Config
- No changes planned (current configuration layering is sufficient).
