//! Input event handling.
//!
//! Handles keyboard and mouse events, routing them to appropriate
//! handlers based on current mode, overlays, and focus.

mod command;
mod event;
mod mouse;
mod overlay;
mod search;
mod terminal;
mod utils;
mod workspace;

const NO_ACTIVE_WORKSPACE_HINT: &str =
    "No active workspace yet. Create one with :workspace create <name>.";

pub use event::handle_event;
pub use search::compute_search_matches;
pub use terminal::{active_search_range, selection_ranges};
pub use workspace::{
    active_tab_mut, active_tab_ref, ensure_active_workspace_tabs, tab_display_label,
};
