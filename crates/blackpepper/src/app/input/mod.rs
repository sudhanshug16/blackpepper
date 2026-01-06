//! Input event handling.
//!
//! Handles keyboard and mouse events, routing them to appropriate
//! handlers based on current mode, overlays, and focus.

mod command;
mod event;
mod mouse;
mod overlay;
mod terminal;
mod utils;
mod workspace;

const NO_ACTIVE_WORKSPACE_HINT: &str =
    "No active workspace yet. Create one with :workspace create <name>.";

pub use event::handle_event;
pub use workspace::ensure_active_workspace_session;
