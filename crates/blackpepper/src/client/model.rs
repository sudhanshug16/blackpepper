use crate::agent_status::AgentState;
use crate::core::{HostId, RegistrySnapshot, RepositoryId, WorkspaceId, WorkspaceSetup};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum GroupKey {
    Repository(RepositoryId),
    Workspace(WorkspaceId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostConnection {
    Local,
    Disconnected,
    Authenticating,
    Connected,
    Reconnecting,
    NeedsAuthentication,
    HostKeyBlocked,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DisplayStatus {
    /// No agent run exists for this workspace. This is intentionally distinct
    /// from `Unknown`, which means a launched agent lost authoritative state.
    Idle,
    Unknown,
    Ready,
    Working,
    Done,
    NeedsInput,
    Exited,
}

impl DisplayStatus {
    pub fn from_agent(state: AgentState) -> Self {
        match state {
            AgentState::Unknown => Self::Unknown,
            AgentState::Working => Self::Working,
            AgentState::NeedsInput => Self::NeedsInput,
            AgentState::Done => Self::Done,
            AgentState::Ready => Self::Ready,
            AgentState::Exited => Self::Exited,
        }
    }

    fn priority(self) -> u8 {
        match self {
            Self::NeedsInput => 6,
            Self::Done => 5,
            Self::Working => 4,
            Self::Ready => 3,
            Self::Unknown => 2,
            Self::Exited => 1,
            Self::Idle => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceNode {
    pub id: WorkspaceId,
    pub label: String,
    pub root_path: String,
    pub status: DisplayStatus,
    pub setup_failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryNode {
    pub id: Option<RepositoryId>,
    pub label: String,
    pub workspaces: Vec<WorkspaceNode>,
    pub status: DisplayStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostNode {
    pub id: HostId,
    pub label: String,
    pub connection: HostConnection,
    pub repositories: Vec<RepositoryNode>,
    pub status: DisplayStatus,
}

pub fn build_tree(
    snapshot: &RegistrySnapshot,
    connections: &BTreeMap<HostId, HostConnection>,
    statuses: &BTreeMap<WorkspaceId, DisplayStatus>,
) -> Vec<HostNode> {
    let mut hosts = snapshot
        .hosts
        .iter()
        .map(|host| {
            let mut groups: BTreeMap<GroupKey, Vec<WorkspaceNode>> = BTreeMap::new();
            for workspace in snapshot
                .workspaces
                .iter()
                .filter(|workspace| workspace.host_id == host.id)
            {
                let label = workspace
                    .display_name
                    .clone()
                    .or_else(|| {
                        Path::new(&workspace.root_path)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                    })
                    .unwrap_or_else(|| workspace.root_path.clone());
                let group = workspace
                    .repository_id()
                    .map(GroupKey::Repository)
                    .unwrap_or(GroupKey::Workspace(workspace.id));
                groups.entry(group).or_default().push(WorkspaceNode {
                    id: workspace.id,
                    label,
                    root_path: workspace.root_path.clone(),
                    status: statuses
                        .get(&workspace.id)
                        .copied()
                        .unwrap_or(DisplayStatus::Idle),
                    setup_failed: matches!(workspace.setup, WorkspaceSetup::Failed { .. }),
                });
            }
            let mut repositories = groups
                .into_iter()
                .map(|(group, mut workspaces)| {
                    workspaces.sort_by(|left, right| left.label.cmp(&right.label));
                    let status = rollup(workspaces.iter().map(|workspace| workspace.status));
                    let (id, label) = match group {
                        GroupKey::Repository(id) => (
                            Some(id),
                            repository_label(snapshot, host.id, id)
                                .unwrap_or_else(|| format!("repo {}", &id.to_string()[..8])),
                        ),
                        GroupKey::Workspace(_) => (None, "folder".to_string()),
                    };
                    RepositoryNode {
                        id,
                        label,
                        workspaces,
                        status,
                    }
                })
                .collect::<Vec<_>>();
            repositories.sort_by(|left, right| left.label.cmp(&right.label));
            let status = rollup(repositories.iter().map(|repository| repository.status));
            HostNode {
                id: host.id,
                label: host.display_name.clone(),
                connection: connections
                    .get(&host.id)
                    .copied()
                    .unwrap_or(HostConnection::Disconnected),
                repositories,
                status,
            }
        })
        .collect::<Vec<_>>();
    hosts.sort_by(|left, right| left.label.cmp(&right.label));
    hosts
}

fn repository_label(
    snapshot: &RegistrySnapshot,
    host_id: HostId,
    repository_id: RepositoryId,
) -> Option<String> {
    let identity = snapshot
        .workspaces
        .iter()
        .find(|workspace| {
            workspace.host_id == host_id && workspace.repository_id() == Some(repository_id)
        })?
        .repository
        .as_ref()?;
    match identity {
        crate::core::RepositoryIdentity::Remote { canonical_url } => {
            let mut parts = canonical_url.split('/');
            let _host = parts.next()?;
            let remainder = parts.collect::<Vec<_>>().join("/");
            (!remainder.is_empty()).then_some(remainder)
        }
        crate::core::RepositoryIdentity::Local { git_common_dir, .. } => {
            let path = Path::new(git_common_dir);
            let repository = if path.file_name().is_some_and(|name| name == ".git") {
                path.parent().unwrap_or(path)
            } else {
                path
            };
            repository
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        }
    }
}

fn rollup(statuses: impl IntoIterator<Item = DisplayStatus>) -> DisplayStatus {
    statuses
        .into_iter()
        .max_by_key(|status| status.priority())
        .unwrap_or(DisplayStatus::Idle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{HostRecord, HostTransport, RepositoryIdentity, WorkspaceRecord};

    #[test]
    fn tree_groups_matching_repositories_and_rolls_up_attention() {
        let host = HostRecord::new("local", HostTransport::Local);
        let repo = RepositoryIdentity::remote("https://github.com/acme/app.git").unwrap();
        let mut first = WorkspaceRecord::new(host.id, "/repo/main");
        first.repository = Some(repo.clone());
        let mut second = WorkspaceRecord::new(host.id, "/repo/feature");
        second.repository = Some(repo);
        let snapshot = RegistrySnapshot {
            hosts: vec![host.clone()],
            workspaces: vec![first.clone(), second.clone()],
            sessions: Vec::new(),
            pending_worktree_removals: Vec::new(),
        };
        let statuses = BTreeMap::from([
            (first.id, DisplayStatus::Working),
            (second.id, DisplayStatus::NeedsInput),
        ]);
        let tree = build_tree(
            &snapshot,
            &BTreeMap::from([(host.id, HostConnection::Local)]),
            &statuses,
        );
        assert_eq!(tree[0].repositories.len(), 1);
        assert_eq!(tree[0].repositories[0].workspaces.len(), 2);
        assert_eq!(tree[0].status, DisplayStatus::NeedsInput);
    }

    #[test]
    fn workspace_without_an_agent_has_no_unknown_warning() {
        let host = HostRecord::new("local", HostTransport::Local);
        let workspace = WorkspaceRecord::new(host.id, "/repo");
        let snapshot = RegistrySnapshot {
            hosts: vec![host.clone()],
            workspaces: vec![workspace],
            sessions: Vec::new(),
            pending_worktree_removals: Vec::new(),
        };

        let tree = build_tree(
            &snapshot,
            &BTreeMap::from([(host.id, HostConnection::Local)]),
            &BTreeMap::new(),
        );

        assert_eq!(tree[0].status, DisplayStatus::Idle);
        assert_eq!(tree[0].repositories[0].status, DisplayStatus::Idle);
        assert_eq!(
            tree[0].repositories[0].workspaces[0].status,
            DisplayStatus::Idle
        );
    }
}
