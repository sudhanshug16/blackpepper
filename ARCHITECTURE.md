# Blackpepper Architecture

Blackpepper is a TUI application that embeds provider UIs (Codex, Claude Code,
OpenCode) inside a terminal interface with an embedded shell per workspace.

## Design Goals

1. **Code clarity over speed** - Decisions and hints embedded in code via comments
2. **Single crate** - All code under `crates/blackpepper/src/`
3. **Provider-agnostic terminal** - Can run any CLI without provider-specific logic
4. **AI-friendly codebase** - Agents can understand decisions at a granular level

## Module Structure

```
crates/blackpepper/src/
├── main.rs              # Entry point, CLI argument handling
├── app/                 # Application orchestration
│   ├── mod.rs           # Module entry, re-exports
│   ├── state.rs         # App struct and type definitions
│   ├── runner.rs        # Main loop, terminal setup/teardown
│   ├── input.rs         # Keyboard/mouse event handling
│   └── render.rs        # UI rendering methods
├── terminal/            # Embedded PTY management
│   ├── mod.rs           # Module entry, re-exports
│   ├── pty.rs           # PTY spawning and session lifecycle
│   ├── render.rs        # VT100 to ratatui rendering
│   ├── input.rs         # Key/mouse event encoding for PTY
│   └── hooks.rs         # Future provider adapter hooks (placeholder)
├── commands/            # Command-mode system (:command)
│   ├── mod.rs           # Module entry, re-exports
│   ├── registry.rs      # Command specs and metadata
│   ├── parse.rs         # Parsing and completion
│   └── exec.rs          # Command execution handlers
├── ui/                  # Pure rendering utilities
│   ├── mod.rs           # Module entry, re-exports
│   ├── layout.rs        # Rect manipulation helpers
│   └── widgets.rs       # Reusable widget builders
├── workspaces/          # Workspace path/naming utilities
│   └── mod.rs
├── git/                 # Git worktree operations
│   └── mod.rs
├── config/              # TOML config loading and merging
│   └── mod.rs
├── state/               # Persistent app state (across sessions)
│   └── mod.rs
├── keymap/              # Key chord parsing and matching
│   └── mod.rs
├── events/              # AppEvent enum for event loop
│   └── mod.rs
└── animals/             # Animal name pool for workspace naming
    └── mod.rs
```

## Key Concepts

### Workspaces

Workspaces are git worktrees created under `./workspaces/<animal>/`. Each workspace
gets a unique animal name (e.g., `otter`, `lynx`) that can be renamed after the first
task is defined. Workspace lifecycle is manual via commands like `:workspace create`,
`:workspace destroy`.

### Tabs

Each workspace can have multiple terminal tabs. Tabs are independent PTY sessions
sharing the same working directory. Tab navigation via Ctrl+Tab, Alt+1-9, or
`:tab switch`.

### Modes

- **Work mode**: Keys go to the terminal (except the toggle chord)
- **Manage mode**: Keys are handled by the app for navigation/commands

Toggle between modes with Ctrl+\ (configurable).

### Command System

Commands follow a `:name subcommand [args]` pattern (vim-like). The registry provides
autocompletion and help text. Some commands run synchronously; workspace operations
run in background threads to avoid blocking the UI.

### Terminal Rendering

The terminal uses `portable-pty` for PTY access and `vt100` for ANSI parsing. The
render pipeline:

1. PTY output → vt100 parser (updates screen buffer)
2. Screen buffer → ratatui Lines (with selection/search highlighting)
3. Lines → frame render

### Configuration

Config resolution order:
1. Workspace-local `.config/blackpepper/pepper.toml`
2. User-level `~/.config/blackpepper/pepper.toml`

Config is TOML-based with sections for keymap, terminal, and workspace settings.

## Extension Points

### Provider Hooks (Future)

`terminal/hooks.rs` defines a `ProviderAdapter` trait for future provider-specific
behavior without polluting core terminal logic. Not yet implemented.

### Commands

New commands can be added by:
1. Adding spec to `commands/registry.rs`
2. Adding handler to `commands/exec.rs`
3. Updating help text

## Build & Test

```sh
cargo build -p blackpepper   # Build
cargo run -p blackpepper     # Run TUI
cargo test -p blackpepper    # Run tests
cargo clippy --workspace -- -D warnings  # Lint
cargo fmt                    # Format
```

## Conventions

- 2021 edition
- `snake_case` for modules/functions, `CamelCase` for types
- Module-level doc comments explaining purpose
- Inline comments for non-obvious decisions
- Conventional commits (`feat:`, `fix:`, `chore:`, etc.)
