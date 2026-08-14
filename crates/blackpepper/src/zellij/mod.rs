//! Pinned Blackpepper Zellij command surface used by workspace sessions.

pub mod appearance;
mod model;
mod runtime;
#[cfg(test)]
mod tests;

pub(crate) use model::classify_pane_process;
pub use model::{
    ClientOperation, PaneProcessState, ZellijClient, ZellijError, ZellijPane, ZellijTab,
};
pub use runtime::{ZellijRuntime, PINNED_VERSION};
