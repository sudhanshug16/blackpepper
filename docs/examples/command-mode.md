# Command Mode

Command mode is a Vim-style interface for workspace and PR actions.

Modes:

- Normal mode keeps focus in the main TUI.
- Control mode enables sidebar controls (toggle with `Ctrl+G`).
- Press `:` in Control mode to open the command line.
- Press `Esc` to close the command line or return to Normal mode.

Examples:

```text
:create
:create otter
:destroy otter
:create-pr
:open-pr
:merge-pr
:help
```

Notes:

- Commands are entered from the sidebar prompt and dispatched per workspace.
- `:create` without a name picks the first unused animal from `src/lib/animals.ts`.
- Available commands are defined in `src/cli/commands.ts`.
