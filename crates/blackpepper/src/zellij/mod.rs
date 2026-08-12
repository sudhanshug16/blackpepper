//! Pinned Zellij 0.44.3 command surface used by Blackpepper workspaces.

mod model;
mod runtime;
#[cfg(test)]
mod tests;

pub(crate) use model::classify_pane_process;
pub use model::{
    ClientOperation, PaneProcessState, ZellijClient, ZellijError, ZellijPane, ZellijTab,
};
pub use runtime::{ZellijRuntime, PINNED_VERSION};
