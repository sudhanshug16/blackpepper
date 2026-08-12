use super::*;
use crate::core::{
    CorePaths, HostRegistry, HostTransport as StoredTransport, RegistrySnapshot, SessionBackend,
    SessionRecord, SingletonLock, WorkspaceRecord,
};
use crate::transport::LocalTransport;
use std::collections::BTreeMap;

struct RuntimeFixture {
    _root: tempfile::TempDir,
    runtime: ClientRuntime,
}

impl RuntimeFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
        paths.prepare().unwrap();
        let singleton = SingletonLock::acquire(paths.singleton_lock_path()).unwrap();
        let mut registry = HostRegistry::open(paths.registry_path()).unwrap();
        let local_host_id = registry.ensure_local_host("test-local").unwrap();
        Self {
            _root: root,
            runtime: ClientRuntime {
                paths,
                registry,
                local_host_id,
                hosts: BTreeMap::from([(local_host_id, HostSlot::Local(LocalTransport))]),
                helper_paths: BTreeMap::new(),
                remote_pending_worktree_removals: BTreeMap::new(),
                local_port_proxies: BTreeMap::new(),
                blocker_watchers: BTreeMap::new(),
                connection_restores: BTreeMap::new(),
                host_operations: BTreeMap::new(),
                host_operation_generations: BTreeMap::new(),
                deferred_host_actions: BTreeMap::new(),
                startup_warnings: Vec::new(),
                _singleton: Some(singleton),
            },
        }
    }

    fn add_live_ssh(&mut self, name: &str, destination: &str) -> HostId {
        let record = HostRecord::new(
            name,
            StoredTransport::Ssh {
                destination: destination.to_owned(),
            },
        );
        self.runtime.registry.upsert_host(&record).unwrap();
        self.runtime.hosts.insert(
            record.id,
            HostSlot::Ssh(Box::new(SshHost {
                alias: name.to_owned(),
                transport: SshTransport::new(SshConfig::new(destination)).unwrap(),
                registry_synchronized: false,
                registry_synchronizing: false,
            })),
        );
        record.id
    }
}

#[test]
fn local_installation_id_collision_is_rejected_without_mutation() {
    let mut fixture = RuntimeFixture::new();
    let connection_id = fixture.add_live_ssh("cloned-vm", "cloned-vm.example");
    fixture
        .runtime
        .helper_paths
        .insert(connection_id, "/existing/helper".to_owned());
    let snapshot_before = fixture.runtime.snapshot().unwrap();
    let helper_paths_before = fixture.runtime.helper_paths.clone();

    let error = validate_remote_host_identity(
        &fixture.runtime,
        connection_id,
        fixture.runtime.local_host_id,
    )
    .unwrap_err();

    assert!(error.contains("local installation ID"));
    assert_eq!(fixture.runtime.snapshot().unwrap(), snapshot_before);
    assert_eq!(fixture.runtime.helper_paths, helper_paths_before);
    assert!(matches!(
        fixture.runtime.hosts.get(&fixture.runtime.local_host_id),
        Some(HostSlot::Local(_))
    ));
    assert!(matches!(
        fixture.runtime.hosts.get(&connection_id),
        Some(HostSlot::Ssh(_))
    ));
}

#[test]
fn a_different_live_ssh_slot_cannot_be_replaced_by_a_colliding_identity() {
    let mut fixture = RuntimeFixture::new();
    let existing_id = fixture.add_live_ssh("primary", "primary.example");
    let connecting_id = fixture.add_live_ssh("clone", "clone.example");
    let snapshot_before = fixture.runtime.snapshot().unwrap();

    let error =
        validate_remote_host_identity(&fixture.runtime, connecting_id, existing_id).unwrap_err();

    assert!(error.contains("different live SSH connection"));
    assert_eq!(fixture.runtime.snapshot().unwrap(), snapshot_before);
    assert!(matches!(
        fixture.runtime.hosts.get(&existing_id),
        Some(HostSlot::Ssh(_))
    ));
    assert!(matches!(
        fixture.runtime.hosts.get(&connecting_id),
        Some(HostSlot::Ssh(_))
    ));
}

#[test]
fn reconnecting_the_same_stable_host_identity_is_allowed() {
    let mut fixture = RuntimeFixture::new();
    let connection_id = fixture.add_live_ssh("primary", "primary.example");

    validate_remote_host_identity(&fixture.runtime, connection_id, connection_id).unwrap();
}

#[test]
fn background_sync_rejects_an_identity_reserved_by_another_live_host() {
    let connection_id = HostId::new();
    let remote_id = HostId::new();
    let error =
        validate_reserved_host_identity(connection_id, remote_id, &BTreeSet::from([remote_id]))
            .unwrap_err();

    assert!(error.contains("another live connection"));
    assert!(validate_reserved_host_identity(
        connection_id,
        connection_id,
        &BTreeSet::from([connection_id])
    )
    .is_ok());
}

#[test]
fn starting_a_second_connection_cannot_replace_a_live_host_slot() {
    let mut fixture = RuntimeFixture::new();
    let connection_id = fixture.add_live_ssh("primary", "primary.example");
    let record = fixture
        .runtime
        .registry
        .host(connection_id)
        .unwrap()
        .unwrap();
    let snapshot_before = fixture.runtime.snapshot().unwrap();
    let (sender, _receiver) = std::sync::mpsc::channel();

    let error = start(&mut fixture.runtime, record, sender).unwrap_err();

    assert!(error.contains("already connecting or connected"));
    assert_eq!(fixture.runtime.snapshot().unwrap(), snapshot_before);
    assert!(matches!(
        fixture.runtime.hosts.get(&connection_id),
        Some(HostSlot::Ssh(_))
    ));
}

#[test]
fn starting_ssh_with_the_local_id_cannot_replace_local_transport() {
    let mut fixture = RuntimeFixture::new();
    let mut record = HostRecord::new(
        "cloned-local",
        StoredTransport::Ssh {
            destination: "cloned-local.example".to_owned(),
        },
    );
    record.id = fixture.runtime.local_host_id;
    let snapshot_before = fixture.runtime.snapshot().unwrap();
    let (sender, _receiver) = std::sync::mpsc::channel();

    let error = start(&mut fixture.runtime, record, sender).unwrap_err();

    assert!(error.contains("cannot replace this client's local host identity"));
    assert_eq!(fixture.runtime.snapshot().unwrap(), snapshot_before);
    assert!(matches!(
        fixture.runtime.hosts.get(&fixture.runtime.local_host_id),
        Some(HostSlot::Local(_))
    ));
}

#[test]
fn workspace_id_collision_is_rejected_without_registry_mutation() {
    let mut fixture = RuntimeFixture::new();
    let remote_id = fixture.add_live_ssh("remote", "remote.example");
    let local = WorkspaceRecord::new(fixture.runtime.local_host_id, "/srv/local");
    fixture.runtime.registry.upsert_workspace(&local).unwrap();
    let mut remote = WorkspaceRecord::new(remote_id, "/srv/remote");
    remote.id = local.id;
    let snapshot_before = fixture.runtime.snapshot().unwrap();

    let error = reconcile_remote_snapshot(
        &mut fixture.runtime,
        remote_id,
        &RegistrySnapshot {
            workspaces: vec![remote],
            ..RegistrySnapshot::default()
        },
    )
    .unwrap_err();

    assert!(error.contains("already belongs to host"));
    assert_eq!(fixture.runtime.snapshot().unwrap(), snapshot_before);
}

#[test]
fn session_id_collision_is_rejected_without_registry_mutation() {
    let mut fixture = RuntimeFixture::new();
    let remote_id = fixture.add_live_ssh("remote", "remote.example");
    let local_workspace = WorkspaceRecord::new(fixture.runtime.local_host_id, "/srv/local");
    fixture
        .runtime
        .registry
        .upsert_workspace(&local_workspace)
        .unwrap();
    let local_session = SessionRecord::new(
        local_workspace.id,
        SessionBackend::Zellij,
        "0.44.3",
        format!("bp-{}", local_workspace.id),
    );
    fixture
        .runtime
        .registry
        .upsert_session(&local_session)
        .unwrap();
    let remote_workspace = WorkspaceRecord::new(remote_id, "/srv/remote");
    let mut remote_session = SessionRecord::new(
        remote_workspace.id,
        SessionBackend::Zellij,
        "0.44.3",
        format!("bp-{}", remote_workspace.id),
    );
    remote_session.id = local_session.id;
    let snapshot_before = fixture.runtime.snapshot().unwrap();

    let error = reconcile_remote_snapshot(
        &mut fixture.runtime,
        remote_id,
        &RegistrySnapshot {
            workspaces: vec![remote_workspace],
            sessions: vec![remote_session],
            ..RegistrySnapshot::default()
        },
    )
    .unwrap_err();

    assert!(error.contains("already belongs to workspace"));
    assert_eq!(fixture.runtime.snapshot().unwrap(), snapshot_before);
}

#[test]
fn remote_pending_removal_ids_survive_client_registry_projection() {
    let mut fixture = RuntimeFixture::new();
    let remote_id = fixture.add_live_ssh("remote", "remote.example");
    let workspace = WorkspaceRecord::new(remote_id, "/srv/pending");
    let remote_snapshot = RegistrySnapshot {
        workspaces: vec![workspace.clone()],
        pending_worktree_removals: vec![workspace.id],
        ..RegistrySnapshot::default()
    };

    reconcile_remote_snapshot(&mut fixture.runtime, remote_id, &remote_snapshot).unwrap();

    let projected = fixture.runtime.snapshot().unwrap();
    assert!(projected.pending_worktree_removals.contains(&workspace.id));
    assert!(projected
        .workspaces
        .iter()
        .any(|candidate| candidate.id == workspace.id));
}

#[test]
fn a_later_remote_snapshot_removes_a_confirmed_deleted_workspace() {
    let mut fixture = RuntimeFixture::new();
    let remote_id = fixture.add_live_ssh("remote", "remote.example");
    let workspace = WorkspaceRecord::new(remote_id, "/srv/removed");
    reconcile_remote_snapshot(
        &mut fixture.runtime,
        remote_id,
        &RegistrySnapshot {
            workspaces: vec![workspace.clone()],
            ..RegistrySnapshot::default()
        },
    )
    .unwrap();

    reconcile_remote_snapshot(
        &mut fixture.runtime,
        remote_id,
        &RegistrySnapshot::default(),
    )
    .unwrap();

    assert!(!fixture
        .runtime
        .snapshot()
        .unwrap()
        .workspaces
        .iter()
        .any(|candidate| candidate.id == workspace.id));
}
