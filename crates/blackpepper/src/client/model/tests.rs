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

#[test]
fn public_status_vocabulary_is_six_fixed_glyph_word_pairs() {
    let rendered = [
        DisplayStatus::Idle,
        DisplayStatus::Ready,
        DisplayStatus::Working,
        DisplayStatus::NeedsInput,
        DisplayStatus::Done,
        DisplayStatus::Exited,
        DisplayStatus::Unknown,
    ]
    .map(DisplayStatus::public_text);

    assert_eq!(
        rendered,
        [
            "· idle",
            "· idle",
            "▸ running",
            "! asks",
            "✓ done",
            "× exited",
            "? unsure",
        ]
    );
}
