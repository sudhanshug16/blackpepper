# Workspace Setup

Workspaces are created with `git worktree` under `./workspaces/<animal>`.

Example flow:

```bash
git worktree add workspaces/otter -b otter
# later, after the first task is defined
# git branch -m otter task-123
```

Notes:

- Keep worktrees outside `node_modules/` and do not nest them.
- Cleanup is manual via command mode (`:destroy`).
- The CLI wraps these via `:create <animal>` and `:destroy <animal>`.
