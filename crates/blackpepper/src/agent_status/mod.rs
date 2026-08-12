//! Provider-neutral agent status tracking.
//!
//! Provider integrations author lifecycle state. A screen-derived blocker
//! overlay may only add a temporary `needs_input` signal; it cannot claim that
//! an agent is working or finished. This keeps an unfamiliar prompt from
//! silently becoming a false completion.

mod blocker;
mod blocker_manifest;
mod model;
mod status_types;
mod store;
mod store_append;
mod store_error;
mod store_freshness;
mod store_fs;
mod store_query;
mod store_schema;
mod tracker;

pub use blocker::{
    BlockerConfidence, BlockerExplain, BlockerInput, BlockerObservation, BlockerOverlay,
};
pub use blocker_manifest::BlockerManifestError;
pub use model::{
    AgentEvent, AgentEventKind, AgentEventSource, AgentSnapshot, AgentState, IntegrationHealth,
    IntegrationIssue, NeedsInputCapability, Provider, ProviderParseError,
};
pub use status_types::{
    AgentExplain, BlockerDisposition, EventDisposition, IgnoredUpdate, StatusAuthority,
};
pub(crate) use store::DeliveryContinuity;
pub use store::{AgentEventDraft, AgentEventStore, StoredAgentUpdate};
pub use store_error::AgentEventStoreError;
pub use tracker::AgentStatusTracker;

#[cfg(test)]
mod blocker_tests;
#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod tracker_tests;
