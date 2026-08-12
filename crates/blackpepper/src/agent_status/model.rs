use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::core::{AgentRunId, HostId, PaneId, WorkspaceId};

/// Coding-agent providers supported by the first status protocol.
#[derive(Debug, Clone, Copy, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Codex,
    Claude,
    #[serde(rename = "opencode")]
    OpenCode,
}

impl Provider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::OpenCode => "opencode",
        }
    }

    /// Screen blockers are the safe baseline when no provider integration is
    /// installed. A healthy integration may advertise a stronger capability.
    pub const fn baseline_needs_input_capability(self) -> NeedsInputCapability {
        match self {
            Self::Codex | Self::Claude | Self::OpenCode => NeedsInputCapability::BlockerOverlay,
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Provider {
    type Err = ProviderParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "claude" | "claude-code" => Ok(Self::Claude),
            "opencode" | "open-code" => Ok(Self::OpenCode),
            _ => Err(ProviderParseError(value.to_string())),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ProviderParseError(String);

impl fmt::Display for ProviderParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unsupported agent provider: {}", self.0)
    }
}

impl std::error::Error for ProviderParseError {}

/// State shown to a Blackpepper client.
///
/// `Done` is never accepted from a provider. The tracker derives it from a
/// completed turn that this client has not marked seen yet.
#[derive(Debug, Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Unknown,
    Working,
    NeedsInput,
    Done,
    #[serde(alias = "idle")]
    Ready,
    Exited,
}

/// How the current provider can detect a prompt that needs a person.
#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NeedsInputCapability {
    Unavailable,
    BlockerOverlay,
    ProviderEvents,
    ProviderEventsWithOverlay,
}

impl NeedsInputCapability {
    pub const fn allows_overlay(self) -> bool {
        matches!(self, Self::BlockerOverlay | Self::ProviderEventsWithOverlay)
    }

    pub const fn accepts_provider_events(self) -> bool {
        matches!(self, Self::ProviderEvents | Self::ProviderEventsWithOverlay)
    }
}

/// Machine-readable integration failures. Free-form provider output is never
/// retained here, so diagnostics cannot leak terminal contents.
#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationIssue {
    ConfigurationConflict,
    InvalidPayload,
    MissingEnvironment,
    TransportUnavailable,
    UnsupportedVersion,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum IntegrationHealth {
    Unknown,
    NotInstalled,
    Starting,
    Healthy { integration_version: Option<u32> },
    Degraded { issue: IntegrationIssue },
    Stale,
}

impl IntegrationHealth {
    pub const fn is_healthy(self) -> bool {
        matches!(self, Self::Healthy { .. })
    }
}

impl<'de> Deserialize<'de> for IntegrationHealth {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, rename_all = "snake_case", tag = "status")]
        enum Wire {
            Unknown {},
            NotInstalled {},
            Starting {},
            Healthy { integration_version: Option<u32> },
            Degraded { issue: IntegrationIssue },
            Stale {},
        }

        Ok(match Wire::deserialize(deserializer)? {
            Wire::Unknown {} => Self::Unknown,
            Wire::NotInstalled {} => Self::NotInstalled,
            Wire::Starting {} => Self::Starting,
            Wire::Healthy {
                integration_version,
            } => Self::Healthy {
                integration_version,
            },
            Wire::Degraded { issue } => Self::Degraded { issue },
            Wire::Stale {} => Self::Stale,
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEventSource {
    ProviderIntegration,
    /// Host-side freshness supervision derived from compact integration
    /// heartbeats. This source never carries provider or terminal payloads.
    IntegrationSupervisor,
    ProcessSupervisor,
}

/// Semantic events accepted from provider adapters and the pane supervisor.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AgentEventKind {
    /// The interactive prompt is ready, but no turn just completed.
    Ready,
    Working,
    NeedsInput,
    /// A turn completed. This advances the client's completion cursor.
    TurnCompleted,
    StateUnknown,
    Exited {
        exit_code: Option<i32>,
    },
    IntegrationHealthChanged {
        health: IntegrationHealth,
    },
}

impl<'de> Deserialize<'de> for AgentEventKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, rename_all = "snake_case", tag = "type")]
        enum Wire {
            Ready {},
            Working {},
            NeedsInput {},
            TurnCompleted {},
            StateUnknown {},
            Exited { exit_code: Option<i32> },
            IntegrationHealthChanged { health: IntegrationHealth },
        }

        Ok(match Wire::deserialize(deserializer)? {
            Wire::Ready {} => Self::Ready,
            Wire::Working {} => Self::Working,
            Wire::NeedsInput {} => Self::NeedsInput,
            Wire::TurnCompleted {} => Self::TurnCompleted,
            Wire::StateUnknown {} => Self::StateUnknown,
            Wire::Exited { exit_code } => Self::Exited { exit_code },
            Wire::IntegrationHealthChanged { health } => Self::IntegrationHealthChanged { health },
        })
    }
}

/// Wire-safe event emitted by the remote status monitor.
#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentEvent {
    pub host_id: HostId,
    pub workspace_id: WorkspaceId,
    pub run_id: AgentRunId,
    pub pane_id: Option<PaneId>,
    pub provider: Provider,
    pub sequence: u64,
    pub observed_at_ms: u64,
    pub source: AgentEventSource,
    pub kind: AgentEventKind,
}

/// Compact state consumed by UI rollups and notifications.
#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSnapshot {
    pub run_id: AgentRunId,
    pub provider: Provider,
    pub state: AgentState,
    pub revision: u64,
    pub completion_revision: u64,
    pub seen_completion_revision: u64,
    pub last_event_sequence: Option<u64>,
    pub last_event_at_ms: Option<u64>,
    pub integration_health: IntegrationHealth,
    pub needs_input_capability: NeedsInputCapability,
    /// A process-supervisor interruption makes the next provider completion
    /// ambiguous. Keep this durable so reconnecting clients cannot turn a
    /// delayed Stop hook into a false `done` state.
    #[serde(default)]
    pub completion_suppressed: bool,
}

impl AgentSnapshot {
    pub const fn has_unseen_completion(&self) -> bool {
        self.completion_revision > self.seen_completion_revision
    }
}
