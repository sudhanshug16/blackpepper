use super::{GroupingPolicy, HostId, RepositoryId, RepositoryIdentity, SessionId, WorkspaceId};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostTransport {
    Local,
    Ssh { destination: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HostRecord {
    pub id: HostId,
    pub display_name: String,
    pub transport: HostTransport,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl HostRecord {
    pub fn new(display_name: impl Into<String>, transport: HostTransport) -> Self {
        let now = now_millis();
        Self {
            id: HostId::new(),
            display_name: display_name.into(),
            transport,
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at_ms = now_millis();
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceRecord {
    pub id: WorkspaceId,
    pub host_id: HostId,
    pub root_path: String,
    pub display_name: Option<String>,
    pub repository: Option<RepositoryIdentity>,
    pub grouping: GroupingPolicy,
    pub setup: WorkspaceSetup,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl WorkspaceRecord {
    pub fn new(host_id: HostId, root_path: impl Into<String>) -> Self {
        let now = now_millis();
        Self {
            id: WorkspaceId::new(),
            host_id,
            root_path: root_path.into(),
            display_name: None,
            repository: None,
            grouping: GroupingPolicy::Automatic,
            setup: WorkspaceSetup::Ready,
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    pub fn repository_id(&self) -> Option<RepositoryId> {
        self.grouping.resolve(self.repository.as_ref())
    }

    pub fn touch(&mut self) {
        self.updated_at_ms = now_millis();
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WorkspaceSetup {
    Ready,
    Failed { message: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum SessionBackend {
    Zellij,
    External(String),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Starting,
    Running,
    Detached,
    Exited,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionRecord {
    pub id: SessionId,
    pub workspace_id: WorkspaceId,
    pub backend: SessionBackend,
    pub backend_version: String,
    pub backend_session_id: String,
    pub state: SessionState,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl SessionRecord {
    pub fn new(
        workspace_id: WorkspaceId,
        backend: SessionBackend,
        backend_version: impl Into<String>,
        backend_session_id: impl Into<String>,
    ) -> Self {
        let now = now_millis();
        Self {
            id: SessionId::new(),
            workspace_id,
            backend,
            backend_version: backend_version.into(),
            backend_session_id: backend_session_id.into(),
            state: SessionState::Starting,
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at_ms = now_millis();
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RegistrySnapshot {
    pub hosts: Vec<HostRecord>,
    pub workspaces: Vec<WorkspaceRecord>,
    pub sessions: Vec<SessionRecord>,
    /// Workspaces retained only so a fresh Worktrunk list can reconcile an
    /// operation whose result became unknown after disconnect. Clients must
    /// not restore sessions or forwards for these IDs.
    #[serde(default)]
    pub pending_worktree_removals: Vec<WorkspaceId>,
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}
