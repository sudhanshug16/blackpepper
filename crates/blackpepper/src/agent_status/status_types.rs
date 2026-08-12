use serde::{Deserialize, Serialize};

use crate::core::AgentRunId;

use super::{
    AgentEventKind, AgentSnapshot, AgentState, BlockerExplain, IntegrationHealth,
    NeedsInputCapability, Provider,
};

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusAuthority {
    None,
    ProviderIntegration,
    IntegrationSupervisor,
    ProcessSupervisor,
    BlockerOverlay,
}

/// Redacted diagnostic state. No terminal text is accepted by this type.
#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentExplain {
    pub run_id: AgentRunId,
    pub provider: Provider,
    pub state: AgentState,
    pub revision: u64,
    pub authority: StatusAuthority,
    pub integration_health: IntegrationHealth,
    pub needs_input_capability: NeedsInputCapability,
    pub completion_revision: u64,
    pub seen_completion_revision: u64,
    pub last_event_sequence: Option<u64>,
    pub last_event_at_ms: Option<u64>,
    pub last_event_kind: Option<AgentEventKind>,
    pub last_blocker_at_ms: Option<u64>,
    pub blocker: Option<BlockerExplain>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum IgnoredUpdate {
    StaleRun,
    ProviderMismatch,
    StaleSequence,
    StaleObservation,
    CapabilityMismatch,
    InvalidSource,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum EventDisposition {
    Applied(AgentSnapshot),
    Ignored(IgnoredUpdate),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum BlockerDisposition {
    Applied {
        snapshot: AgentSnapshot,
        changed: bool,
    },
    Ignored(IgnoredUpdate),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum BaseState {
    Unknown,
    Working,
    NeedsInput,
    Idle,
    Exited,
}
