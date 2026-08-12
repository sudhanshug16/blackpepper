use serde::{Deserialize, Serialize};

use crate::agent_status::{
    BlockerConfidence, BlockerExplain, BlockerObservation, IntegrationHealth, Provider,
};
use crate::core::{AgentRunId, HostId, PaneId, WorkspaceId};

/// Stable context attached to every redacted transition.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct MonitorContext {
    pub host_id: HostId,
    pub workspace_id: WorkspaceId,
    pub run_id: AgentRunId,
    pub pane_id: PaneId,
    pub provider: Provider,
    pub integration_health: IntegrationHealth,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockerSource {
    ZellijViewport,
}

/// A screen rule may only add or clear a temporary needs-input overlay.
#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "change")]
pub enum BlockerChange {
    NeedsInput {
        rule_id: String,
        confidence: BlockerConfidence,
        priority: i32,
    },
    Cleared,
}

/// Compact output safe to persist or send over SSH.
///
/// There is intentionally no prompt, viewport, line, command, or evidence
/// field in this type.
#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BlockerTransition {
    pub host_id: HostId,
    pub workspace_id: WorkspaceId,
    pub run_id: AgentRunId,
    pub pane_id: PaneId,
    pub provider: Provider,
    pub sequence: u64,
    pub observed_at_ms: u64,
    pub source: BlockerSource,
    pub manifest_version: String,
    pub state: BlockerChange,
}

impl BlockerTransition {
    /// Convert the host-scoped wire update into the tracker overlay input.
    pub fn observation(&self) -> BlockerObservation {
        BlockerObservation {
            run_id: self.run_id,
            sequence: self.sequence,
            observed_at_ms: self.observed_at_ms,
            blocker: match &self.state {
                BlockerChange::NeedsInput {
                    rule_id,
                    confidence,
                    priority,
                } => Some(BlockerExplain {
                    provider: self.provider,
                    manifest_version: self.manifest_version.clone(),
                    rule_id: rule_id.clone(),
                    confidence: *confidence,
                    priority: *priority,
                }),
                BlockerChange::Cleared => None,
            },
        }
    }
}

/// Non-sensitive counters returned when a subscription stream ends.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct StreamStats {
    pub lines: u64,
    pub malformed: u64,
    pub oversize: u64,
    pub unknown_events: u64,
    pub ignored_other_panes: u64,
    pub transitions: u64,
}
