use super::{provisional_attachment_count, ClientRuntime};
use crate::client::runtime::HostSlot;
use crate::core::{CorePaths, HostRegistry, SingletonLock};
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
    let root = tempfile::tempdir().unwrap();
    let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
    paths.prepare().unwrap();
    let singleton = SingletonLock::acquire(paths.singleton_lock_path()).unwrap();
    let mut registry = HostRegistry::open(paths.registry_path()).unwrap();
    let local_host_id = registry.ensure_local_host("test-local").unwrap();
    let mut runtime = ClientRuntime {
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
    let missing = root.path().join("sidecars/zellij/0.44.3/zellij");

    assert!(!runtime
        .binary_matches(local_host_id, missing.to_str().unwrap(), "0.44.3")
        .unwrap());
}
