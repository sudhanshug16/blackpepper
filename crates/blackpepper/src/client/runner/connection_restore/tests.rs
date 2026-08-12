use super::{disconnected_hosts, generation::GenerationGate, Coordinator};
use crate::client::runtime::ClientRuntime;
use crate::client::{ClientEvent, ClientState, HostConnection};
use crate::core::HostId;
use crate::transport::CommandCancellation;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Barrier};

#[test]
fn newer_generation_rejects_the_previous_result() {
    let host_id = HostId::new();
    let mut gate = GenerationGate::default();
    let (old_generation, old_token) = gate.begin(host_id);
    let (new_generation, new_token) = gate.begin(host_id);

    assert!(!gate.is_current(host_id, old_generation, old_token));
    assert!(gate.is_current(host_id, new_generation, new_token));
    assert!(!gate.finish(host_id, old_generation, old_token));
    assert!(gate.finish(host_id, new_generation, new_token));
}

#[test]
fn stalled_restore_worker_cannot_block_raw_input_events() {
    let host_id = HostId::new();
    let token = uuid::Uuid::new_v4();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let worker_entered = Arc::clone(&entered);
    let worker_release = Arc::clone(&release);
    let (events, receiver) = mpsc::channel();
    let event_sender = events.clone();
    let worker = std::thread::spawn(move || {
        worker_entered.wait();
        worker_release.wait();
        let _ = event_sender.send(ClientEvent::ConnectionRestoreComplete { token, host_id });
    });
    entered.wait();

    events.send(ClientEvent::RawInput(vec![b'x'])).unwrap();
    assert!(matches!(
        receiver.recv().unwrap(),
        ClientEvent::RawInput(bytes) if bytes == b"x"
    ));

    release.wait();
    worker.join().unwrap();
}

#[test]
fn shutdown_cancels_and_joins_restore_worker() {
    let host_id = HostId::new();
    let mut coordinator = Coordinator::default();
    let (generation, token) = coordinator.gate.begin(host_id);
    let cancellation = CommandCancellation::default();
    let worker_cancellation = cancellation.clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let worker_stopped = Arc::clone(&stopped);
    let (_outcome_tx, outcome_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        while !worker_cancellation.is_cancelled() {
            std::thread::yield_now();
        }
        worker_stopped.store(true, Ordering::SeqCst);
    });
    coordinator.attach_test_job(host_id, generation, token, cancellation, outcome_rx, worker);

    coordinator.shutdown();

    assert!(stopped.load(Ordering::SeqCst));
    assert!(coordinator.jobs.is_empty());
}

#[test]
fn stable_id_disconnect_after_worker_outcome_invalidates_temp_generation() {
    let temporary_id = HostId::new();
    let stable_id = HostId::new();
    let mut coordinator = Coordinator::default();
    let (generation, token) = coordinator.gate.begin(temporary_id);
    let cancellation = CommandCancellation::default();
    let (_outcome_tx, outcome_rx) = mpsc::channel();
    // This worker represents an already-produced outcome whose completion
    // event has not yet been dispatched by the UI loop.
    let worker = std::thread::spawn(|| {});
    coordinator.attach_test_job(
        temporary_id,
        generation,
        token,
        cancellation.clone(),
        outcome_rx,
        worker,
    );

    let invalidated = disconnected_hosts(
        &coordinator.jobs,
        |host_id| host_id == temporary_id,
        |host_id, observed_token| host_id == temporary_id && observed_token == token,
    );
    for host_id in invalidated {
        coordinator.invalidate(host_id);
    }

    assert!(cancellation.is_cancelled());
    assert!(!coordinator.gate.is_current(temporary_id, generation, token));
    assert_ne!(temporary_id, stable_id);
}

#[test]
fn worker_spawn_failure_drops_no_payload_and_leaves_host_reconnectable() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = ClientRuntime::test_fixture(root.path());
    let host_id = runtime.test_add_ssh_slot("devbox", "devbox.invalid");
    let (events, _receiver) = mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        runtime.snapshot().unwrap(),
        events.clone(),
    );
    state
        .connections
        .insert(host_id, HostConnection::Reconnecting);
    let mut coordinator = Coordinator::default();

    coordinator.start_with_spawner(&mut state, &mut runtime, host_id, &events, |_worker| {
        Err(std::io::Error::other("injected spawn failure"))
    });

    assert_eq!(
        state.connections.get(&host_id),
        Some(&HostConnection::Failed)
    );
    assert!(runtime.test_connection_can_start(host_id));
    assert!(coordinator.jobs.is_empty());
    assert!(state
        .output
        .as_deref()
        .is_some_and(|output| output.contains("injected spawn failure")));
}
