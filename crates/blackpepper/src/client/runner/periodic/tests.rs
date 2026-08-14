use super::{
    apply::cleanup_target_index, complete, invalidate_host, invalidate_owned, schedule,
    spawn_refresh_waiter, Coordinator,
};
use crate::client::runtime::{ClientRuntime, HostOperationContext, HostOperationValue};
use crate::client::ClientEvent;
use crate::client::{ClientState, HostConnection};
use crate::core::{HostId, HostPeriodicRefresh, RegistrySnapshot, WorkspaceRecord};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Barrier};

mod metadata;

#[test]
fn coordinator_coalesces_and_rejects_a_stale_connection_generation() {
    let host_id = HostId::new();
    let mut coordinator = Coordinator::default();
    let token = coordinator.begin(host_id, Vec::new()).unwrap();
    assert!(coordinator.begin(host_id, Vec::new()).is_none());
    assert!(coordinator.invalidate(host_id).is_empty());
    assert!(!coordinator.finish(host_id, token));
    assert!(coordinator.begin(host_id, Vec::new()).is_some());
}

#[test]
fn stalled_refresh_worker_cannot_block_terminal_input_events() {
    let host_id = HostId::new();
    let token = uuid::Uuid::new_v4();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let worker_entered = Arc::clone(&entered);
    let worker_release = Arc::clone(&release);
    let (sender, receiver) = mpsc::channel();
    let (_cancellation, worker) = spawn_refresh_waiter(
        token,
        host_id,
        move |_cancellation| {
            worker_entered.wait();
            worker_release.wait();
            Err("simulated dead host".to_owned())
        },
        sender.clone(),
    );
    entered.wait();

    sender.send(ClientEvent::RawInput(vec![b'x'])).unwrap();
    assert!(matches!(receiver.recv().unwrap(), ClientEvent::RawInput(bytes) if bytes == b"x"));

    release.wait();
    worker.join().unwrap();
    assert!(matches!(
        receiver.recv().unwrap(),
        ClientEvent::PeriodicRefreshComplete {
            host_id: completed_host,
            result: Err(_),
            ..
        } if completed_host == host_id
    ));
}

#[test]
fn shutdown_cancels_and_joins_a_stalled_refresh_worker() {
    let host_id = HostId::new();
    let mut coordinator = Coordinator::default();
    let token = coordinator.begin(host_id, Vec::new()).unwrap();
    let entered = Arc::new(Barrier::new(2));
    let worker_entered = Arc::clone(&entered);
    let stopped = Arc::new(AtomicBool::new(false));
    let worker_stopped = Arc::clone(&stopped);
    let (events, _receiver) = mpsc::channel();
    let (cancellation, worker) = spawn_refresh_waiter(
        token,
        host_id,
        move |cancellation| {
            worker_entered.wait();
            cancellation.recv().unwrap();
            worker_stopped.store(true, Ordering::SeqCst);
            Err("cancelled".to_owned())
        },
        events,
    );
    coordinator.attach_worker(host_id, token, cancellation, worker);
    entered.wait();

    coordinator.shutdown();

    assert!(stopped.load(Ordering::SeqCst));
    assert!(coordinator.in_flight.is_empty());
}

#[test]
fn stale_cleanup_result_cannot_cancel_a_replacement_forward() {
    let host_id = HostId::new();
    let workspace_id = crate::core::WorkspaceId::new();
    let mut old = crate::ports::ForwardState::new(
        host_id,
        workspace_id,
        crate::ports::RemotePortTarget {
            remote_host: "127.0.0.1".to_owned(),
            remote_port: 8080,
        },
    )
    .unwrap();
    old.status = crate::ports::ForwardStatus::Cancelling;
    let old_id = old.id;
    let replacement = crate::ports::ForwardState::new(
        host_id,
        workspace_id,
        crate::ports::RemotePortTarget {
            remote_host: "127.0.0.1".to_owned(),
            remote_port: 8080,
        },
    )
    .unwrap();

    assert_eq!(cleanup_target_index(&[old], old_id, host_id), Some(0));
    assert_eq!(cleanup_target_index(&[replacement], old_id, host_id), None);
}

#[test]
fn periodic_refresh_skips_a_host_owned_by_a_stalled_explicit_operation() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = ClientRuntime::test_fixture(root.path());
    let host_id = runtime.local_host_id();
    let (events, receiver) = mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        RegistrySnapshot {
            hosts: runtime.snapshot().unwrap().hosts,
            ..RegistrySnapshot::default()
        },
        events.clone(),
    );
    state.connections.insert(host_id, HostConnection::Local);
    runtime
        .start_host_operation(
            host_id,
            "stalled agent handshake",
            HostOperationContext::Terminate {
                workspace_id: crate::core::WorkspaceId::new(),
            },
            events.clone(),
            Box::new(|_| {
                while !crate::transport::CommandCancellation::scope_is_cancelled() {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Ok(HostOperationValue::Terminated)
            }),
        )
        .unwrap();
    let mut coordinator = Coordinator::default();

    schedule(&mut state, &mut runtime, &mut coordinator, &events);

    assert!(coordinator.in_flight.is_empty());
    assert!(receiver.try_recv().is_err());
    runtime.cancel_host_operation(host_id).unwrap();
}

#[test]
fn refresh_started_before_explicit_operation_cannot_apply_or_clean_forwards_later() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = ClientRuntime::test_fixture(root.path());
    let host_id = runtime.test_add_ssh_slot("remote", "remote.example");
    let workspace = WorkspaceRecord::new(host_id, "/srv/project");
    runtime.registry.upsert_workspace(&workspace).unwrap();
    let (events, receiver) = mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        runtime.snapshot().unwrap(),
        events.clone(),
    );
    state.connections.insert(host_id, HostConnection::Connected);
    let forward = crate::ports::ForwardState::new(
        host_id,
        workspace.id,
        crate::ports::RemotePortTarget {
            remote_host: "127.0.0.1".to_owned(),
            remote_port: 31_337,
        },
    )
    .unwrap();
    let forward_id = forward.id;
    state.forwards.push(forward);

    let mut coordinator = Coordinator::default();
    let refresh_token = coordinator.begin(host_id, Vec::new()).unwrap();
    let operation_token = runtime
        .start_host_operation(
            host_id,
            "explicit mutation",
            HostOperationContext::Terminate {
                workspace_id: workspace.id,
            },
            events,
            Box::new(|_| {
                while !crate::transport::CommandCancellation::scope_is_cancelled() {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Ok(HostOperationValue::Terminated)
            }),
        )
        .unwrap();
    invalidate_owned(&mut state, &runtime, &mut coordinator);
    runtime.cancel_host_operation(host_id).unwrap();
    let (completed_token, generation) = loop {
        match receiver
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap()
        {
            ClientEvent::HostOperationComplete {
                token, generation, ..
            } => break (token, generation),
            _ => continue,
        }
    };
    assert_eq!(completed_token, operation_token);
    runtime
        .finish_host_operation(host_id, generation, operation_token)
        .unwrap();
    assert!(!runtime.host_is_owned_by_background_work(host_id));

    complete(
        &mut state,
        &mut runtime,
        &mut coordinator,
        refresh_token,
        host_id,
        Ok(Box::new(HostPeriodicRefresh {
            host_id,
            registry: RegistrySnapshot::default(),
            ports: crate::ports::failed_probe("stale refresh must not apply"),
            agent_runs: Vec::new(),
            agent_snapshots: Default::default(),
            agent_observation_errors: Default::default(),
            watchable_agent_runs: Vec::new(),
            connected_clients: Default::default(),
            client_count_errors: Default::default(),
            errors: vec!["stale refresh must not apply".to_owned()],
            overviews: Default::default(),
        })),
    );

    assert!(state
        .snapshot
        .workspaces
        .iter()
        .any(|candidate| candidate.id == workspace.id));
    assert_eq!(state.forwards.len(), 1);
    assert_eq!(state.forwards[0].id, forward_id);
    assert_eq!(
        state.forwards[0].status,
        crate::ports::ForwardStatus::Active
    );
    assert!(!coordinator.in_flight.contains_key(&host_id));
}

#[test]
fn invalidated_cleanup_restores_only_its_exact_forward_to_retryable_state() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = ClientRuntime::test_fixture(root.path());
    let host_id = runtime.test_add_ssh_slot("remote", "remote.example");
    let workspace_id = crate::core::WorkspaceId::new();
    let mut target = crate::ports::ForwardState::new(
        host_id,
        workspace_id,
        crate::ports::RemotePortTarget {
            remote_host: "127.0.0.1".to_owned(),
            remote_port: 31_337,
        },
    )
    .unwrap();
    target.status = crate::ports::ForwardStatus::Cancelling;
    let target_id = target.id;
    let replacement = crate::ports::ForwardState::new(
        host_id,
        workspace_id,
        crate::ports::RemotePortTarget {
            remote_host: "127.0.0.1".to_owned(),
            remote_port: 31_337,
        },
    )
    .unwrap();
    let replacement_id = replacement.id;
    let (events, _receiver) = mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        runtime.snapshot().unwrap(),
        events,
    );
    state.forwards = vec![target, replacement];
    let mut coordinator = Coordinator::default();
    let token = coordinator.begin(host_id, vec![target_id]).unwrap();

    invalidate_host(&mut state, &mut coordinator, host_id);

    assert!(!coordinator.finish(host_id, token));
    assert!(matches!(
        &state.forwards[0].status,
        crate::ports::ForwardStatus::Failed(message)
            if message.contains("will be retried")
    ));
    assert_eq!(state.forwards[1].id, replacement_id);
    assert_eq!(
        state.forwards[1].status,
        crate::ports::ForwardStatus::Active
    );
}
