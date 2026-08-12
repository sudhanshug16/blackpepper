use super::*;
use crate::core::{SessionBackend, SessionRecord, SessionState};

#[test]
fn approved_remove_refuses_any_non_exited_zellij_session() {
    let fixture = RemovalFixture::new();
    let executor = fixture.executor();
    let preview = executor
        .remove(
            &fixture.registry,
            fixture.target.id,
            &fixture.target.root_path,
            None,
        )
        .unwrap();
    let HostServicePayload::WorktrunkApprovalRequired { approval, .. } = preview else {
        panic!("expected approval preview");
    };
    let mut session = SessionRecord::new(
        fixture.target.id,
        SessionBackend::Zellij,
        "0.44.3",
        format!("bp-{}", fixture.target.id),
    );
    session.state = SessionState::Detached;
    fixture.registry.upsert_session(&session).unwrap();

    let error = executor
        .remove(
            &fixture.registry,
            fixture.target.id,
            &fixture.target.root_path,
            Some(&approval),
        )
        .unwrap_err();
    assert!(error.contains("non-exited Zellij session"));
    assert!(error.contains("Detached"));
    assert!(!fixture.remove_marker.exists());
    assert!(fixture
        .registry
        .worktrunk_removal(fixture.target.id)
        .unwrap()
        .is_none());
}

#[test]
fn session_creation_that_wins_the_lease_makes_remove_refuse() {
    use super::super::super::session_lease::SessionInitializationLease;
    use std::sync::mpsc;
    use std::time::Duration;

    let fixture = RemovalFixture::new();
    let preview = fixture
        .executor()
        .remove(
            &fixture.registry,
            fixture.target.id,
            &fixture.target.root_path,
            None,
        )
        .unwrap();
    let HostServicePayload::WorktrunkApprovalRequired { approval, .. } = preview else {
        panic!("expected approval preview");
    };
    let lease =
        SessionInitializationLease::acquire_for_workspace(&fixture.paths, fixture.target.id)
            .unwrap();
    let registry_path = fixture.paths.registry_path();
    let paths = fixture.paths.clone();
    let binary = fixture.binary.clone();
    let workspace_id = fixture.target.id;
    let target_path = fixture.target.root_path.clone();
    let (result_tx, result_rx) = mpsc::channel();
    let remover = std::thread::spawn(move || {
        let registry = HostRegistry::open(registry_path).unwrap();
        let result = WorktrunkExecutor::with_binary(&paths, binary).remove(
            &registry,
            workspace_id,
            &target_path,
            Some(&approval),
        );
        result_tx.send(result).unwrap();
    });
    assert!(
        result_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "remove must wait behind the session lifecycle lease"
    );

    let mut session = SessionRecord::new(
        fixture.target.id,
        SessionBackend::Zellij,
        "0.44.3",
        format!("bp-{}", fixture.target.id),
    );
    session.state = SessionState::Running;
    fixture.registry.upsert_session(&session).unwrap();
    drop(lease);

    let error = result_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("remover finished")
        .unwrap_err();
    remover.join().unwrap();
    assert!(error.contains("non-exited Zellij session"));
    assert!(!fixture.remove_marker.exists());
}
