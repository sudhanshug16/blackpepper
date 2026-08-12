use crate::agent_status::{AgentExplain, AgentSnapshot, StoredAgentUpdate};
use crate::ports::PortSnapshot;
use crate::worktrunk::{
    SwitchResult, WorktreeList, WorktrunkApprovalToken, WorktrunkProjectCommand,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use super::super::{
    AgentRunId, HostId, PaneId, RepositoryIdentity, SessionId, WorkspaceId, WorkspaceRecord,
};
use crate::agent_status::Provider;

/// Repository details safe to share with a client. The original remote URL is
/// omitted because it may contain embedded credentials.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryInspection {
    pub identity: RepositoryIdentity,
    pub git_common_dir: PathBuf,
}

/// Host-scoped context kept separately from provider hook payloads.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostAgentSnapshot {
    pub host_id: HostId,
    pub workspace_id: WorkspaceId,
    pub pane_id: Option<PaneId>,
    pub snapshot: AgentSnapshot,
    pub explain: AgentExplain,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostAgentUpdate {
    pub host_id: HostId,
    pub workspace_id: WorkspaceId,
    pub pane_id: Option<PaneId>,
    pub update: StoredAgentUpdate,
}

/// One coalesced host observation returned to the standalone client. All
/// fields are compact metadata: terminal contents, provider payloads, and
/// commands are deliberately absent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostPeriodicRefresh {
    pub host_id: HostId,
    pub registry: crate::core::RegistrySnapshot,
    pub ports: PortSnapshot,
    pub agent_runs: Vec<HostAgentRun>,
    pub agent_snapshots: BTreeMap<AgentRunId, HostAgentSnapshot>,
    pub watchable_agent_runs: Vec<AgentRunId>,
    pub connected_clients: BTreeMap<WorkspaceId, usize>,
    pub client_count_errors: BTreeMap<WorkspaceId, String>,
    pub errors: Vec<String>,
    /// Per-workspace detail only the host can compute. Defaulted so a client
    /// built against this field still reads a refresh from an older helper.
    #[serde(default)]
    pub overviews: BTreeMap<WorkspaceId, WorkspaceOverview>,
}

/// Repository and session context for one workspace, gathered host-side
/// because the checkout and the Zellij session both live there.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceOverview {
    /// Branch name, or `detached`. `None` when the folder is not a checkout.
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub dirty: bool,
    #[serde(default)]
    pub ahead: u32,
    #[serde(default)]
    pub behind: u32,
    #[serde(default)]
    pub pull_request: Option<PullRequestSummary>,
    /// One-based position of the focused tab and the session's tab count.
    #[serde(default)]
    pub active_tab: Option<u32>,
    #[serde(default)]
    pub tab_count: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PullRequestSummary {
    pub number: u32,
    pub state: PullRequestState,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestState {
    Open,
    Draft,
    Merged,
    Closed,
}

impl PullRequestState {
    pub const fn word(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Draft => "draft",
            Self::Merged => "merged",
            Self::Closed => "closed",
        }
    }
}

/// Exact Zellij identity recorded after an agent tab and its terminal pane
/// have both been observed. Rehydration must match every field before it can
/// treat a process as the original run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRunBinding {
    pub session_id: SessionId,
    pub session_name: String,
    pub zellij_version: String,
    pub tab_id: u64,
    pub tab_name: String,
    pub zellij_pane_id: String,
}

/// Compact, semantic run state shared between clients. This intentionally has
/// no provider payload, command, terminal text, or managed-asset contents.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostAgentRun {
    pub host_id: HostId,
    pub workspace_id: WorkspaceId,
    pub run_id: AgentRunId,
    pub pane_id: PaneId,
    pub provider: Provider,
    pub binding: AgentRunBinding,
    pub snapshot: AgentSnapshot,
}

/// A client-side observation of the exact bound Zellij tab and pane.
/// `Missing` is distinct on the wire for diagnostics, but is terminal for the
/// recorded run just like an observed process exit.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "state")]
pub enum AgentProcessObservation {
    Live,
    StateUnknown,
    Missing,
    Exited { exit_code: Option<i32> },
}

/// Successful Worktrunk mutation shapes exposed by the helper.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorktrunkMutationOutcome {
    Switched { result: SwitchResult },
    Removed { path: PathBuf },
    SetupFailed { path: PathBuf, message: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostServicePayload {
    Ports {
        snapshot: PortSnapshot,
    },
    PeriodicRefresh {
        refresh: Box<HostPeriodicRefresh>,
    },
    RepositoryInspected {
        repository: Option<RepositoryInspection>,
    },
    WorkspaceRegistered {
        workspace: WorkspaceRecord,
    },
    AgentSnapshot {
        snapshot: Option<HostAgentSnapshot>,
    },
    AgentUpdates {
        updates: Vec<HostAgentUpdate>,
    },
    AgentRuns {
        runs: Vec<HostAgentRun>,
    },
    AgentRunReconciled {
        run: Box<HostAgentRun>,
    },
    Worktrees {
        list: WorktreeList,
    },
    WorktrunkApprovalRequired {
        command: String,
        approval: WorktrunkApprovalToken,
        unapproved_project_commands: Vec<WorktrunkProjectCommand>,
    },
    WorktrunkMutation {
        outcome: WorktrunkMutationOutcome,
    },
}
