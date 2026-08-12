mod server;
mod service_types;
mod wire;

pub use server::{serve_json_lines, serve_json_lines_with_extension, ProtocolExtension};
pub use service_types::{
    AgentProcessObservation, AgentRunBinding, HostAgentRun, HostAgentSnapshot, HostAgentUpdate,
    HostPeriodicRefresh, HostServicePayload, RepositoryInspection, WorktrunkMutationOutcome,
};
pub use wire::ProtocolError;

use super::{
    AgentRunId, HostId, PaneId, RegistrySnapshot, SessionId, SessionRecord, WorkspaceId,
    WorkspaceRecord,
};
use crate::agent_status::Provider;
use crate::worktrunk::WorktrunkApprovalToken;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HelperRequest {
    pub request_id: u64,
    pub protocol_version: u32,
    #[serde(flatten)]
    pub operation: RequestOperation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    deny_unknown_fields,
    tag = "method",
    content = "params",
    rename_all = "snake_case"
)]
pub enum RequestOperation {
    Handshake {
        client_version: String,
    },
    Snapshot,
    // Host mutations stay client-local: an SSH alias must never become shared remote truth.
    UpsertWorkspace {
        workspace: WorkspaceRecord,
    },
    UpsertSession {
        session: SessionRecord,
    },
    RemoveWorkspace {
        workspace_id: WorkspaceId,
    },
    RemoveSession {
        session_id: SessionId,
    },
    DiscoverPorts,
    /// Read-only, host-scoped observations used by the client's periodic UI
    /// refresh. Keeping these observations in one transient helper invocation
    /// lets the client wait for the entire bounded operation off its render
    /// thread without weakening the SSH ControlMaster boundary.
    PeriodicRefresh {
        attached_workspaces: Vec<WorkspaceId>,
    },
    InspectRepository {
        root_path: String,
    },
    RegisterWorkspace {
        root_path: String,
        display_name: Option<String>,
    },
    AgentSnapshot {
        run_id: AgentRunId,
    },
    AgentFollow {
        run_id: AgentRunId,
        after_sequence: u64,
        limit: usize,
    },
    RegisterAgentRun {
        workspace_id: WorkspaceId,
        run_id: AgentRunId,
        pane_id: Option<PaneId>,
        provider: Provider,
    },
    BindAgentRun {
        workspace_id: WorkspaceId,
        run_id: AgentRunId,
        pane_id: PaneId,
        provider: Provider,
        binding: AgentRunBinding,
    },
    AbortAgentRun {
        workspace_id: WorkspaceId,
        run_id: AgentRunId,
        pane_id: PaneId,
        provider: Provider,
    },
    ListAgentRuns {
        workspace_id: Option<WorkspaceId>,
    },
    ReconcileAgentRun {
        workspace_id: WorkspaceId,
        run_id: AgentRunId,
        pane_id: PaneId,
        provider: Provider,
        binding: AgentRunBinding,
        observation: AgentProcessObservation,
    },
    WorktrunkList {
        workspace_id: WorkspaceId,
        repository_path: String,
    },
    WorktrunkCreate {
        repository_path: String,
        branch: String,
        base: Option<String>,
        approval: Option<WorktrunkApprovalToken>,
    },
    WorktrunkSwitch {
        repository_path: String,
        selector: String,
        approval: Option<WorktrunkApprovalToken>,
    },
    WorktrunkRemove {
        workspace_id: WorkspaceId,
        target_path: String,
        approval: Option<WorktrunkApprovalToken>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HelperResponse {
    pub request_id: Option<u64>,
    pub protocol_version: u32,
    #[serde(flatten)]
    pub result: ResponseResult,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ResponseResult {
    Ok { payload: ResponsePayload },
    Error { error: ProtocolFailure },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResponsePayload {
    Handshake {
        helper_version: String,
        protocol_version: u32,
        host_id: HostId,
    },
    Snapshot {
        snapshot: RegistrySnapshot,
    },
    Acknowledged,
    Removed {
        existed: bool,
    },
    HostService {
        payload: Box<HostServicePayload>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProtocolFailure {
    pub code: FailureCode,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    InvalidRequest,
    VersionMismatch,
    HandshakeRequired,
    RegistryError,
    HostServiceError,
    UnsupportedOperation,
}

#[cfg(test)]
mod tests;
