use super::*;

fn test_runtime() -> (tempfile::TempDir, ClientRuntime) {
    let root = tempfile::tempdir().unwrap();
    let paths =
        crate::core::CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
    paths.prepare().unwrap();
    let singleton = crate::core::SingletonLock::acquire(paths.singleton_lock_path()).unwrap();
    let mut registry = crate::core::HostRegistry::open(paths.registry_path()).unwrap();
    let local_host_id = registry.ensure_local_host("test-local").unwrap();
    let runtime = ClientRuntime {
        paths,
        registry,
        local_host_id,
        hosts: std::collections::BTreeMap::from([(
            local_host_id,
            super::super::HostSlot::Local(crate::transport::LocalTransport),
        )]),
        helper_paths: std::collections::BTreeMap::new(),
        remote_pending_worktree_removals: std::collections::BTreeMap::new(),
        local_port_proxies: std::collections::BTreeMap::new(),
        blocker_watchers: std::collections::BTreeMap::new(),
        connection_restores: std::collections::BTreeMap::new(),
        host_operations: std::collections::BTreeMap::new(),
        host_operation_generations: std::collections::BTreeMap::new(),
        deferred_host_actions: std::collections::BTreeMap::new(),
        startup_warnings: Vec::new(),
        _singleton: Some(singleton),
    };
    (root, runtime)
}

fn forward(host_id: HostId, workspace_id: WorkspaceId, status: ForwardStatus) -> ForwardState {
    ForwardState {
        id: uuid::Uuid::new_v4(),
        host_id,
        workspace_id,
        remote_host: "127.0.0.1".to_string(),
        remote_port: 4321,
        requested_local_address: "127.0.0.1:54321".parse().unwrap(),
        local_address: "127.0.0.1:54321".parse().unwrap(),
        status,
    }
}

#[test]
fn workspace_port_filter_accepts_descendants_but_not_prefix_siblings() {
    assert!(listener_matches_workspace(
        Some(Path::new("/srv/app")),
        "/srv/app"
    ));
    assert!(listener_matches_workspace(
        Some(Path::new("/srv/app/packages/api")),
        "/srv/app"
    ));
    assert!(!listener_matches_workspace(
        Some(Path::new("/srv/application")),
        "/srv/app"
    ));
}

#[test]
fn ssh_forward_keeps_the_discovered_remote_address() {
    let forward = ForwardState {
        id: uuid::Uuid::new_v4(),
        host_id: HostId::new(),
        workspace_id: WorkspaceId::new(),
        remote_host: "::1".to_string(),
        remote_port: 4321,
        requested_local_address: "127.0.0.1:54321".parse().unwrap(),
        local_address: "127.0.0.1:54321".parse().unwrap(),
        status: ForwardStatus::Active,
    };

    let transport_forward = local_forward(&forward);
    assert_eq!(transport_forward.remote_host, "::1");
    assert_eq!(transport_forward.remote_port, 4321);
    assert_eq!(transport_forward.local_port, 54321);
}

#[test]
fn cancellation_does_not_require_the_workspace_registry_row() {
    let (_root, mut runtime) = test_runtime();
    let removed_workspace = WorkspaceId::new();
    let forward = forward(
        runtime.local_host_id,
        removed_workspace,
        ForwardStatus::Active,
    );

    runtime
        .cancel_workspace_forward(removed_workspace, &forward)
        .unwrap();
}

#[test]
fn reconciliation_purges_only_forwards_without_a_matching_workspace_and_host() {
    let (_root, mut runtime) = test_runtime();
    let workspace = crate::core::WorkspaceRecord::new(runtime.local_host_id, "/srv/live");
    runtime.registry.upsert_workspace(&workspace).unwrap();
    let mismatched_host = HostId::new();
    let mut forwards = vec![
        forward(runtime.local_host_id, workspace.id, ForwardStatus::Active),
        forward(
            runtime.local_host_id,
            WorkspaceId::new(),
            ForwardStatus::Active,
        ),
        forward(mismatched_host, workspace.id, ForwardStatus::Active),
    ];
    let snapshot = runtime.snapshot().unwrap();

    let report = runtime.reconcile_forwards(&mut forwards, &snapshot);

    assert_eq!(report.removed, 2);
    assert!(report.failures.is_empty());
    assert_eq!(forwards.len(), 1);
    assert_eq!(forwards[0].workspace_id, workspace.id);
    assert_eq!(forwards[0].host_id, runtime.local_host_id);
}

#[test]
fn pending_worktrunk_removal_is_not_a_restorable_forward_owner() {
    let (_root, mut runtime) = test_runtime();
    let workspace = crate::core::WorkspaceRecord::new(runtime.local_host_id, "/srv/pending");
    runtime.registry.upsert_workspace(&workspace).unwrap();
    runtime.remote_pending_worktree_removals.insert(
        runtime.local_host_id,
        std::collections::BTreeSet::from([workspace.id]),
    );
    let mut forwards = vec![forward(
        runtime.local_host_id,
        workspace.id,
        ForwardStatus::Reconnecting,
    )];
    let snapshot = runtime.snapshot().unwrap();

    let report = runtime.reconcile_forwards(&mut forwards, &snapshot);

    assert_eq!(report.removed, 1);
    assert!(forwards.is_empty());

    let mut stale = forward(
        runtime.local_host_id,
        workspace.id,
        ForwardStatus::Reconnecting,
    );
    runtime.reconnect_forward(&mut stale);
    assert!(matches!(stale.status, ForwardStatus::Failed(_)));
}

#[test]
fn pending_worktrunk_removal_refuses_a_new_forward() {
    let (_root, mut runtime) = test_runtime();
    let workspace = crate::core::WorkspaceRecord::new(runtime.local_host_id, "/srv/pending");
    runtime.registry.upsert_workspace(&workspace).unwrap();
    runtime.remote_pending_worktree_removals.insert(
        runtime.local_host_id,
        std::collections::BTreeSet::from([workspace.id]),
    );

    let error = runtime
        .forward_workspace_port(
            workspace.id,
            RemotePortTarget::from_bind_address("127.0.0.1", 4321).unwrap(),
        )
        .unwrap_err();

    assert!(error.contains("unknown result"));
}

#[test]
fn approved_removal_cancels_all_target_forwards_and_keeps_other_workspaces() {
    let (_root, mut runtime) = test_runtime();
    let target = WorkspaceId::new();
    let survivor = WorkspaceId::new();
    let mut forwards = vec![
        forward(runtime.local_host_id, target, ForwardStatus::Active),
        forward(runtime.local_host_id, target, ForwardStatus::Active),
        forward(runtime.local_host_id, survivor, ForwardStatus::Active),
    ];

    let cancelled = runtime
        .cancel_workspace_forwards(&mut forwards, target)
        .unwrap();

    assert_eq!(cancelled, 2);
    assert_eq!(forwards.len(), 1);
    assert_eq!(forwards[0].workspace_id, survivor);
}

#[test]
fn approved_removal_is_blocked_when_a_transport_cannot_cancel_its_forward() {
    let (_root, mut runtime) = test_runtime();
    let remote = crate::core::HostRecord::new(
        "remote",
        crate::core::HostTransport::Ssh {
            destination: "remote.example".to_owned(),
        },
    );
    runtime.registry.upsert_host(&remote).unwrap();
    runtime.hosts.insert(
        remote.id,
        super::super::HostSlot::Ssh(Box::new(super::super::SshHost {
            alias: "remote".to_owned(),
            transport: crate::transport::SshTransport::new(crate::transport::SshConfig::new(
                "remote.example",
            ))
            .unwrap(),
            registry_synchronized: true,
            registry_synchronizing: false,
        })),
    );
    let target = WorkspaceId::new();
    let mut forwards = vec![forward(remote.id, target, ForwardStatus::Active)];

    let error = runtime
        .cancel_workspace_forwards(&mut forwards, target)
        .unwrap_err();

    assert!(error.contains("removal was blocked"));
    assert_eq!(forwards.len(), 1);
    assert!(matches!(forwards[0].status, ForwardStatus::Failed(_)));
}

#[test]
fn reconnect_refuses_a_removed_local_workspace() {
    let (_root, mut runtime) = test_runtime();
    let mut forward = forward(
        runtime.local_host_id,
        WorkspaceId::new(),
        ForwardStatus::Reconnecting,
    );

    runtime.reconnect_forward(&mut forward);

    assert!(matches!(forward.status, ForwardStatus::Failed(_)));
}
