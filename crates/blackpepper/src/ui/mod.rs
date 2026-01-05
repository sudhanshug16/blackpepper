//! UI rendering and layout utilities.
//!
//! This module contains pure rendering logic separated from state.
//! All functions here take data and produce ratatui widgets without
//! side effects. This makes the UI easier to test and reason about.
//!
//! Submodules:
//! - layout: helpers for rect manipulation and centering
//! - widgets: reusable widget builders (overlays, loaders, etc.)

mod layout;
mod widgets;

pub use layout::{centered_rect, inset_horizontal};
pub use widgets::{render_loader, render_overlay, render_separator};
