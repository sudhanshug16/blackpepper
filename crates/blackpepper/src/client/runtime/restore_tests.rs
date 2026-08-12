use super::*;
use crate::core::{CorePaths, HostId, HostRecord, HostRegistry, HostTransport, SingletonLock};
use crate::transport::CommandCancellation;
use std::collections::BTreeMap;

fn test_runtime() -> (tempfile::TempDir, ClientRuntime) {
    let root = tempfile::tempdir().unwrap();
    let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
    paths.prepare().unwrap();
    let singleton = SingletonLock::acquire(paths.singleton_lock_path()).unwrap();
    let mut registry = HostRegistry::open(paths.registry_path()).unwrap();
    let local_host_id = registry.ensure_local_host("local").unwrap();
    let runtime = ClientRuntime {
        paths,
        registry,
        local_host_id,
        hosts: BTreeMap::from([(
            local_host_id,
            HostSlot::Local(crate::transport::LocalTransport),
        )]),
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

#[test]
fn stable_id_disconnect_cancels_temporary_restore_generation() {
    let (_root, mut runtime) = test_runtime();
    let temporary = HostId::new();
    let stable = HostId::new();
    let destination = "remote.example".to_owned();
    let token = uuid::Uuid::new_v4();
    let cancellation = CommandCancellation::default();
    runtime.connection_restores.insert(
        temporary,
        ActiveConnectionRestore {
            token,
            destination: destination.clone(),
            cancellation: cancellation.clone(),
        },
    );
    let mut stable_record = HostRecord::new(
        "remote",
        HostTransport::Ssh {
            destination: destination.clone(),
        },
    );
    stable_record.id = stable;
    runtime.registry.upsert_host(&stable_record).unwrap();

    let connection_ids = runtime.cancel_connection_restores(stable);

    assert_eq!(connection_ids, vec![temporary]);
    assert!(cancellation.is_cancelled());
    assert!(runtime.connection_restore_matches(&stable_record));
}

#[test]
fn split_does_not_wait_for_the_registry_initialization_lock() {
    use fs2::FileExt;
    use std::time::{Duration, Instant};

    let (_root, mut runtime) = test_runtime();
    let host_id = runtime.test_add_ssh_slot("devbox", "devbox.invalid");
    let mut lock_path = runtime.paths.registry_path().as_os_str().to_owned();
    lock_path.push(".init.lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(std::path::PathBuf::from(lock_path))
        .unwrap();
    lock.lock_exclusive().unwrap();
    let cancellation = CommandCancellation::default();
    let token = uuid::Uuid::new_v4();

    let started = Instant::now();
    let restored = runtime
        .split_connection_restore(host_id, token, cancellation)
        .unwrap();

    assert!(started.elapsed() < Duration::from_millis(100));
    drop(restored);
    runtime.forget_connection_restore(host_id, token);
}
