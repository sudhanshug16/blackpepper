//! Application orchestration and main event loop.
//!
//! This module owns the core application lifecycle:
//! - Initialization (terminal setup, config loading)
//! - Event loop (input, PTY output, commands)
//! - State management (workspaces, tabs, overlays)
//! - UI rendering delegation
//!
//! The app is structured around a single `App` struct that holds
//! all state. Events are processed sequentially in the main loop.
//!
//! Submodules:
//! - state: App struct and type definitions
//! - runner: main loop and terminal setup
//! - input: keyboard and mouse event handling
//! - render: UI rendering methods

mod input;
mod render;
mod runner;
mod state;

pub use runner::run;
