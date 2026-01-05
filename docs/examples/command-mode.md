# Command Mode

Command mode is a Vim-style interface for workspace and PR actions.

Modes:

- Work mode keeps focus in the embedded terminal.
- Manage mode enables global controls (toggle with `Ctrl+G`).
- Press `Ctrl+P` to open the workspace switcher overlay.
- Press `:` in Manage mode to open the command line (hidden by default).
- Press `Esc` to close the command line or return to Work mode.

Examples:

```text
:create
:create otter
:destroy otter
:workspace
:workspace otter
:create-pr
:open-pr
:merge-pr
:help
:quit
:q
```

Notes:

- Commands are entered from the command bar at the bottom.
- `:create` without a name picks the first unused animal from `crates/blackpepper/src/animals.rs`.
- `:workspace` without a name opens the workspace switcher overlay.
- Selecting a workspace starts an embedded terminal for that worktree.
- Quit with `:quit` (or `:q`) or press `q` while in Manage mode.
- Available commands are defined in `crates/blackpepper/src/commands.rs`.
