use super::super::{ClientRuntime, SpawnedAgent, WorktreeChange};
use crate::agent_status::Provider;
use crate::client::ClientCommand;
use crate::core::{HostId, WorkspaceId, WorkspaceRecord};
use crate::ports::ForwardState;
use crate::transport::PtyProcess;
use std::path::PathBuf;

pub(super) type HostOperationWork =
    Box<dyn FnOnce(&mut ClientRuntime) -> Result<HostOperationValue, String> + Send + 'static>;

pub(crate) enum HostOperationContext {
    DurableState,
    SshImportPreview,
    AgentSpawn {
        workspace_id: WorkspaceId,
        provider: Provider,
    },
    ServiceStart {
        workspace_id: WorkspaceId,
        name: String,
    },
    WorktreeList {
        workspace_id: WorkspaceId,
    },
    WorktreeMutation {
        workspace_id: WorkspaceId,
        command: ClientCommand,
        replaces_forwards: bool,
    },
    StatusExplain {
        workspace_id: WorkspaceId,
    },
    PortList {
        host_id: HostId,
        all_host: bool,
    },
    ForwardStart {
        workspace_id: WorkspaceId,
    },
    ForwardCancel {
        workspace_id: WorkspaceId,
        forward_id: uuid::Uuid,
    },
    Attach {
        workspace_id: WorkspaceId,
    },
    RegisterAndAttach {
        host_id: HostId,
        path: PathBuf,
    },
    InitialShellFocus {
        workspace_id: WorkspaceId,
    },
    WorkspaceUngroup {
        workspace_id: WorkspaceId,
    },
    Terminate {
        workspace_id: WorkspaceId,
    },
}

pub(crate) enum HostOperationValue {
    DurableState(Vec<DeferredHostResult>),
    SshImportPreview(Vec<String>),
    AgentSpawned(SpawnedAgent),
    ServiceStarted {
        tab_id: u64,
    },
    Worktrees(crate::worktrunk::WorktreeList),
    WorktreeMutation(WorktreeMutationResult),
    AgentDiagnostics {
        snapshots: Vec<(
            crate::core::AgentRunId,
            Result<Option<crate::core::HostAgentSnapshot>, String>,
        )>,
    },
    Ports {
        snapshot: crate::ports::PortSnapshot,
    },
    Forwarded(ForwardState),
    ForwardCancelled(ForwardState),
    Attached {
        workspace_id: WorkspaceId,
        process: PtyProcess,
        provisional_clients: usize,
    },
    RegisteredAndAttached {
        workspace_id: WorkspaceId,
        path: PathBuf,
        attachment: Result<(PtyProcess, usize), String>,
    },
    InitialShellFocused,
    WorkspaceUngrouped(WorkspaceRecord),
    Terminated,
}

pub(crate) struct WorktreeMutationResult {
    pub change: Result<WorktreeChange, String>,
    /// Present only for approved removal. It is the authoritative remainder
    /// after attempting to stop every client-owned forward.
    pub forwards: Option<Vec<ForwardState>>,
    pub session_error: Option<String>,
}

pub(crate) struct CompletedHostOperation {
    pub host_id: HostId,
    pub label: String,
    pub context: HostOperationContext,
    pub result: Result<HostOperationValue, String>,
    pub snapshot: Result<crate::core::RegistrySnapshot, String>,
    pub deferred_results: Vec<DeferredHostResult>,
    pub deferred_remaining: Vec<DeferredHostAction>,
    pub discarded: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum DeferredHostAction {
    MarkDetached {
        workspace_id: WorkspaceId,
    },
    MarkAgentsUnknown {
        workspace_id: WorkspaceId,
        run_ids: Vec<crate::core::AgentRunId>,
    },
}

#[derive(Debug)]
pub(crate) enum DeferredHostResult {
    Detached {
        workspace_id: WorkspaceId,
        result: Result<(), String>,
    },
    AgentsUnknown {
        workspace_id: WorkspaceId,
        results: Vec<(
            crate::core::AgentRunId,
            Result<crate::core::HostAgentRun, String>,
        )>,
    },
}
