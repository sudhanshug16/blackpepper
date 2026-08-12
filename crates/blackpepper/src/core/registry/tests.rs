use super::*;
use crate::core::{
    GroupingPolicy, HostTransport, RepositoryIdentity, SessionBackend, SessionState,
};
use std::fs;
use std::sync::{Arc, Barrier};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[test]
fn round_trips_records_and_cascades_host_removal() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private/registry.sqlite3");
    let registry = HostRegistry::open(&path).unwrap();
    let host = HostRecord::new("local", HostTransport::Local);
    let mut workspace = WorkspaceRecord::new(host.id, "/srv/pepper");
    workspace.grouping = GroupingPolicy::Ungrouped;
    let mut session = SessionRecord::new(workspace.id, SessionBackend::Zellij, "0.44.3", "pepper");
    session.state = SessionState::Running;

    registry.upsert_host(&host).unwrap();
    registry.upsert_workspace(&workspace).unwrap();
    registry.upsert_session(&session).unwrap();

    assert_eq!(registry.journal_mode().unwrap(), "wal");
    assert_eq!(registry.snapshot().unwrap().hosts, vec![host.clone()]);
    assert_eq!(
        registry.workspace(workspace.id).unwrap(),
        Some(workspace.clone())
    );
    assert_eq!(registry.session(session.id).unwrap(), Some(session));
    assert_eq!(
        registry.sessions_for_workspace(workspace.id).unwrap().len(),
        1
    );

    assert!(registry.remove_host(host.id).unwrap());
    let snapshot = registry.snapshot().unwrap();
    assert!(snapshot.hosts.is_empty());
    assert!(snapshot.workspaces.is_empty());
    assert!(snapshot.sessions.is_empty());
}

#[test]
fn local_host_identity_is_stable() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("registry.sqlite3");
    let first = {
        let mut registry = HostRegistry::open(&path).unwrap();
        registry.ensure_local_host("server-a").unwrap()
    };
    let second = {
        let mut registry = HostRegistry::open(&path).unwrap();
        registry.ensure_local_host("renamed-server").unwrap()
    };
    assert_eq!(first, second);
}

#[test]
fn concurrent_first_open_converges_on_one_local_host() {
    let root = tempfile::tempdir().unwrap();
    let path = Arc::new(root.path().join("registry.sqlite3"));
    let barrier = Arc::new(Barrier::new(4));
    let handles = (0..4)
        .map(|_| {
            let path = Arc::clone(&path);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                let mut registry = HostRegistry::open(path.as_ref()).unwrap();
                registry.ensure_local_host("server-a").unwrap()
            })
        })
        .collect::<Vec<_>>();
    let ids = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert!(ids.iter().all(|id| *id == ids[0]));
}

#[test]
fn rejects_cross_host_local_repository_identity() {
    let root = tempfile::tempdir().unwrap();
    let registry = HostRegistry::open(root.path().join("registry.sqlite3")).unwrap();
    let host = HostRecord::new("local", HostTransport::Local);
    registry.upsert_host(&host).unwrap();
    let mut workspace = WorkspaceRecord::new(host.id, "/srv/pepper");
    workspace.repository =
        Some(RepositoryIdentity::local(HostId::new(), "/srv/pepper/.git").unwrap());
    assert!(matches!(
        registry.upsert_workspace(&workspace),
        Err(RegistryError::Validation(_))
    ));
}

#[test]
fn rejects_noncanonical_remote_that_could_persist_credentials() {
    let root = tempfile::tempdir().unwrap();
    let registry = HostRegistry::open(root.path().join("registry.sqlite3")).unwrap();
    let host = HostRecord::new("local", HostTransport::Local);
    registry.upsert_host(&host).unwrap();
    let mut workspace = WorkspaceRecord::new(host.id, "/srv/pepper");
    workspace.repository = Some(RepositoryIdentity::Remote {
        canonical_url: "https://token@github.com/acme/pepper.git".to_owned(),
    });
    assert!(matches!(
        registry.upsert_workspace(&workspace),
        Err(RegistryError::Validation(_))
    ));
}

#[test]
fn rejects_session_without_a_backend_version() {
    let root = tempfile::tempdir().unwrap();
    let registry = HostRegistry::open(root.path().join("registry.sqlite3")).unwrap();
    let mut session = SessionRecord::new(WorkspaceId::new(), SessionBackend::Zellij, "", "pepper");
    session.state = SessionState::Running;
    assert!(matches!(
        registry.upsert_session(&session),
        Err(RegistryError::Validation(_))
    ));
}

#[test]
fn worktrunk_removal_journal_is_exact_and_finishes_atomically() {
    let root = tempfile::tempdir().unwrap();
    let mut registry = HostRegistry::open(root.path().join("registry.sqlite3")).unwrap();
    let host_id = registry.ensure_local_host("server-a").unwrap();
    let identity = RepositoryIdentity::remote("https://github.com/acme/pepper.git").unwrap();
    let mut survivor = WorkspaceRecord::new(host_id, "/srv/pepper");
    survivor.repository = Some(identity.clone());
    let mut target = WorkspaceRecord::new(host_id, "/srv/pepper-feature");
    target.repository = Some(identity);
    registry.upsert_workspace(&survivor).unwrap();
    registry.upsert_workspace(&target).unwrap();

    let intent = registry
        .plan_worktrunk_removal(
            target.id,
            survivor.id,
            &target.root_path,
            "/srv/pepper/.git".to_owned(),
        )
        .unwrap();
    registry.journal_worktrunk_removal(&intent).unwrap();
    assert_eq!(
        registry.worktrunk_removal(target.id).unwrap(),
        Some(intent.clone())
    );
    assert_eq!(
        registry.snapshot().unwrap().pending_worktree_removals,
        vec![target.id]
    );
    assert!(registry.journal_worktrunk_removal(&intent).is_err());

    assert!(registry.finish_worktrunk_removal(&intent).unwrap());
    assert!(registry.workspace(target.id).unwrap().is_none());
    assert!(registry.workspace(survivor.id).unwrap().is_some());
    assert!(registry.worktrunk_removal(target.id).unwrap().is_none());
    assert!(registry
        .snapshot()
        .unwrap()
        .pending_worktree_removals
        .is_empty());
}

#[test]
fn worktrunk_removal_marker_survives_an_independent_workspace_delete() {
    let root = tempfile::tempdir().unwrap();
    let mut registry = HostRegistry::open(root.path().join("registry.sqlite3")).unwrap();
    let host_id = registry.ensure_local_host("server-a").unwrap();
    let identity = RepositoryIdentity::remote("git@github.com:acme/pepper.git").unwrap();
    let mut survivor = WorkspaceRecord::new(host_id, "/srv/pepper");
    survivor.repository = Some(identity.clone());
    let mut target = WorkspaceRecord::new(host_id, "/srv/pepper-feature");
    target.repository = Some(identity);
    registry.upsert_workspace(&survivor).unwrap();
    registry.upsert_workspace(&target).unwrap();
    let intent = registry
        .plan_worktrunk_removal(
            target.id,
            survivor.id,
            &target.root_path,
            "/srv/pepper/.git".to_owned(),
        )
        .unwrap();
    registry.journal_worktrunk_removal(&intent).unwrap();

    assert!(registry.remove_workspace(target.id).unwrap());
    assert_eq!(
        registry.worktrunk_removal(target.id).unwrap(),
        Some(intent.clone())
    );
    assert!(!registry.finish_worktrunk_removal(&intent).unwrap());
    assert!(registry.worktrunk_removal(target.id).unwrap().is_none());
}

#[test]
fn worktrunk_removal_rejects_stale_paths_and_repository_mismatches() {
    let root = tempfile::tempdir().unwrap();
    let mut registry = HostRegistry::open(root.path().join("registry.sqlite3")).unwrap();
    let host_id = registry.ensure_local_host("server-a").unwrap();
    let mut survivor = WorkspaceRecord::new(host_id, "/srv/pepper");
    survivor.repository =
        Some(RepositoryIdentity::remote("https://github.com/acme/pepper.git").unwrap());
    let mut target = WorkspaceRecord::new(host_id, "/srv/other-feature");
    target.repository =
        Some(RepositoryIdentity::remote("https://github.com/acme/other.git").unwrap());
    registry.upsert_workspace(&survivor).unwrap();
    registry.upsert_workspace(&target).unwrap();

    assert!(registry
        .plan_worktrunk_removal(
            target.id,
            survivor.id,
            "/stale/path",
            "/srv/pepper/.git".to_owned(),
        )
        .is_err());
    assert!(registry
        .plan_worktrunk_removal(
            target.id,
            survivor.id,
            &target.root_path,
            "/srv/pepper/.git".to_owned(),
        )
        .is_err());
}

#[cfg(unix)]
#[test]
fn registry_and_parent_are_private() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private/registry.sqlite3");
    let _registry = HostRegistry::open(&path).unwrap();
    assert_eq!(
        fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
        0o700
    );
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    assert_eq!(
        fs::metadata(path.with_extension("sqlite3.init.lock"))
            .unwrap()
            .mode()
            & 0o777,
        0o600
    );
}
