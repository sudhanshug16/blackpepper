use super::*;
use crate::core::{RepositoryIdentity, WorkspaceRecord};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

#[test]
fn parser_requires_each_stable_identity_once() {
    let workspace_id = WorkspaceId::new();
    let session_name = format!("bp-{workspace_id}");
    let parsed = SessionLeaseArgs::parse([
        "--workspace-id".to_owned(),
        workspace_id.to_string(),
        "--session".to_owned(),
        session_name.clone(),
    ])
    .unwrap();
    assert_eq!(parsed.workspace_id, workspace_id);
    assert_eq!(parsed.session_name, session_name);
    assert!(SessionLeaseArgs::parse([
        "--workspace-id".to_owned(),
        workspace_id.to_string(),
        "--workspace-id".to_owned(),
        workspace_id.to_string(),
    ])
    .is_none());
}

#[test]
fn contention_times_out_and_the_lock_file_is_private() {
    let root = tempfile::tempdir().unwrap();
    let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
    paths.prepare().unwrap();
    let workspace_id = WorkspaceId::new();
    let session_name = format!("bp-{workspace_id}");
    let _first = SessionInitializationLease::acquire(
        &paths,
        workspace_id,
        &session_name,
        Duration::from_secs(1),
    )
    .unwrap();

    let started = Instant::now();
    let error = SessionInitializationLease::acquire(
        &paths,
        workspace_id,
        &session_name,
        Duration::from_millis(75),
    )
    .err()
    .expect("second lease must time out");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(error.contains("still changing"));

    #[cfg(unix)]
    assert_eq!(
        fs::metadata(lease_path(&paths, workspace_id, &session_name))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn two_clients_can_start_the_session_only_once() {
    let root = tempfile::tempdir().unwrap();
    let paths = Arc::new(CorePaths::from_roots(
        root.path().join("state"),
        root.path().join("run"),
    ));
    paths.prepare().unwrap();
    let workspace_id = WorkspaceId::new();
    let session_name = format!("bp-{workspace_id}");
    let barrier = Arc::new(Barrier::new(3));
    let session_exists = Arc::new(AtomicBool::new(false));
    let starts = Arc::new(AtomicUsize::new(0));

    let clients = (0..2)
        .map(|_| {
            let paths = Arc::clone(&paths);
            let barrier = Arc::clone(&barrier);
            let session_exists = Arc::clone(&session_exists);
            let starts = Arc::clone(&starts);
            let session_name = session_name.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let _lease = SessionInitializationLease::acquire(
                    &paths,
                    workspace_id,
                    &session_name,
                    Duration::from_secs(2),
                )
                .unwrap();
                if !session_exists.swap(true, Ordering::SeqCst) {
                    starts.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(50));
                }
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    for client in clients {
        client.join().unwrap();
    }

    assert_eq!(starts.load(Ordering::SeqCst), 1);
}

#[test]
fn pending_worktrunk_removal_refuses_session_recreation() {
    let root = tempfile::tempdir().unwrap();
    let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
    paths.prepare().unwrap();
    let mut registry = HostRegistry::open(paths.registry_path()).unwrap();
    let host_id = registry.ensure_local_host("test-host").unwrap();
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

    let arguments = SessionLeaseArgs {
        workspace_id: target.id,
        session_name: format!("bp-{}", target.id),
    };
    let error = hold_session_lease(&paths, &registry, &arguments, std::io::empty(), Vec::new())
        .unwrap_err();
    assert!(error.contains("unknown result"));
}
