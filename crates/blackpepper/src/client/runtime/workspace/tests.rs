use super::{
    provisional_attachment_count,
    session::{latest_non_exited_zellij_session, zellij_session_name},
    ClientRuntime,
};
use crate::client::runtime::HostSlot;
use crate::core::{
    CorePaths, HostRegistry, SessionBackend, SessionRecord, SessionState, SingletonLock,
    WorkspaceRecord,
};
use crate::transport::LocalTransport;
use std::collections::BTreeMap;

#[test]
fn provisional_attach_count_includes_the_client_being_started() {
    assert_eq!(provisional_attachment_count(0), 1);
    assert_eq!(provisional_attachment_count(2), 3);
    assert_eq!(provisional_attachment_count(usize::MAX), usize::MAX);
}

#[test]
fn a_missing_local_sidecar_is_a_cache_miss() {
    let (root, mut runtime) = local_runtime();
    let local_host_id = runtime.local_host_id;
    let missing = root.path().join("sidecars/zellij/0.44.3/zellij");

    assert!(!runtime
        .binary_matches(local_host_id, missing.to_str().unwrap(), "0.44.3")
        .unwrap());
}

#[test]
fn existing_session_keeps_its_recorded_zellij_generation() {
    let (_root, runtime) = local_runtime();
    let workspace = WorkspaceRecord::new(runtime.local_host_id, "/tmp/private-zellij-test");
    runtime.registry.upsert_workspace(&workspace).unwrap();
    let session = SessionRecord::new(
        workspace.id,
        SessionBackend::Zellij,
        crate::transport::LEGACY_ZELLIJ_VERSION,
        format!("bp-{}", workspace.id),
    );
    runtime.registry.upsert_session(&session).unwrap();

    let selected = runtime.current_or_new_session(&workspace).unwrap();

    assert_eq!(selected.id, session.id);
    assert_eq!(
        selected.backend_version,
        crate::transport::LEGACY_ZELLIJ_VERSION
    );
    assert_eq!(selected.backend_session_id, format!("bp-{}", workspace.id));
}

#[test]
fn workspace_without_a_live_session_uses_the_current_zellij_generation() {
    let (_root, runtime) = local_runtime();
    let workspace = WorkspaceRecord::new(runtime.local_host_id, "/tmp/private-zellij-new");

    let selected = runtime.current_or_new_session(&workspace).unwrap();

    assert_eq!(selected.backend_version, crate::transport::ZELLIJ_VERSION);
    assert_eq!(
        selected.backend_session_id,
        zellij_session_name(workspace.id, crate::transport::ZELLIJ_VERSION)
    );
}

#[test]
fn exited_current_generation_reuses_its_registry_identity_on_reopen() {
    let (_root, runtime) = local_runtime();
    let workspace = WorkspaceRecord::new(runtime.local_host_id, "/tmp/private-zellij-reopen");
    runtime.registry.upsert_workspace(&workspace).unwrap();
    let mut exited = SessionRecord::new(
        workspace.id,
        SessionBackend::Zellij,
        crate::transport::ZELLIJ_VERSION,
        zellij_session_name(workspace.id, crate::transport::ZELLIJ_VERSION),
    );
    exited.state = SessionState::Exited;
    runtime.registry.upsert_session(&exited).unwrap();

    let mut reopened = runtime.current_or_new_session(&workspace).unwrap();

    assert_eq!(reopened.id, exited.id);
    assert_eq!(reopened.state, SessionState::Exited);
    reopened.state = SessionState::Starting;
    reopened.touch();
    runtime.registry.upsert_session(&reopened).unwrap();
    assert_eq!(
        runtime
            .registry
            .sessions_for_workspace(workspace.id)
            .unwrap(),
        [reopened]
    );
}

#[test]
fn newer_exited_history_does_not_shadow_the_current_zellij_session() {
    let host_id = crate::core::HostId::new();
    let workspace = WorkspaceRecord::new(host_id, "/tmp/private-zellij-selection");
    let mut current = SessionRecord::new(
        workspace.id,
        SessionBackend::Zellij,
        crate::transport::ZELLIJ_VERSION,
        "bp-current",
    );
    current.state = SessionState::Running;
    current.created_at_ms = 10;
    current.updated_at_ms = 10;
    let mut newer_external = SessionRecord::new(
        workspace.id,
        SessionBackend::External("fixture".to_owned()),
        "1",
        "external-newer",
    );
    newer_external.state = SessionState::Running;
    newer_external.created_at_ms = 20;
    newer_external.updated_at_ms = 20;
    let mut newest_exited = SessionRecord::new(
        workspace.id,
        SessionBackend::Zellij,
        "0.44.3-blackpepper.2",
        "bp-newest-exited",
    );
    newest_exited.state = SessionState::Exited;
    newest_exited.created_at_ms = 30;
    newest_exited.updated_at_ms = 30;

    let sessions = [current.clone(), newer_external, newest_exited.clone()];

    assert_eq!(
        latest_non_exited_zellij_session(&sessions).map(|session| session.id),
        Some(current.id)
    );
    assert!(latest_non_exited_zellij_session(&[newest_exited]).is_none());
}

#[test]
fn stock_zellij_session_name_keeps_the_legacy_identity() {
    let (_root, runtime) = local_runtime();
    let workspace = WorkspaceRecord::new(runtime.local_host_id, "/tmp/stock-zellij-name");

    assert_eq!(
        zellij_session_name(workspace.id, crate::transport::LEGACY_ZELLIJ_VERSION),
        format!("bp-{}", workspace.id)
    );
}

#[test]
fn branded_zellij_generations_get_distinct_stable_session_names() {
    let (_root, runtime) = local_runtime();
    let workspace = WorkspaceRecord::new(runtime.local_host_id, "/tmp/branded-zellij-name");
    let first_version = "0.44.3-blackpepper.1";
    let next_version = crate::transport::PATCHED_ZELLIJ_VERSION;
    let first = zellij_session_name(workspace.id, first_version);
    let first_again = zellij_session_name(workspace.id, first_version);
    let next = zellij_session_name(workspace.id, next_version);

    assert_eq!(first, first_again);
    assert_ne!(first, format!("bp-{}", workspace.id));
    assert_ne!(first, next);
    assert!(first.ends_with(&crate::transport::sha256_bytes(first_version.as_bytes())[..12]));
    assert!(next.ends_with(&crate::transport::sha256_bytes(next_version.as_bytes())[..12]));
    assert!(first.len() <= 64);
    assert!(next.len() <= 64);
}

fn local_runtime() -> (tempfile::TempDir, ClientRuntime) {
    let root = tempfile::tempdir().unwrap();
    let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
    paths.prepare().unwrap();
    let singleton = SingletonLock::acquire(paths.singleton_lock_path()).unwrap();
    let mut registry = HostRegistry::open(paths.registry_path()).unwrap();
    let local_host_id = registry.ensure_local_host("test-local").unwrap();
    let runtime = ClientRuntime {
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
    };
    (root, runtime)
}
