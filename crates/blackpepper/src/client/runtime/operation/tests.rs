//! Deterministic host-operation coordinator tests.

use super::*;
use crate::client::{ClientCommand, ClientEvent};
use crate::core::WorkspaceId;
use std::sync::{mpsc, Arc, Barrier};
use std::time::{Duration, Instant};

fn context() -> HostOperationContext {
    HostOperationContext::Terminate {
        workspace_id: WorkspaceId::new(),
    }
}

fn fixture() -> (tempfile::TempDir, ClientRuntime) {
    let root = tempfile::tempdir().unwrap();
    let runtime = ClientRuntime::test_fixture(root.path());
    (root, runtime)
}

fn completion(receiver: &mpsc::Receiver<ClientEvent>) -> (uuid::Uuid, HostId, u64) {
    loop {
        match receiver.recv_timeout(Duration::from_secs(3)).unwrap() {
            ClientEvent::HostOperationComplete {
                token,
                host_id,
                generation,
            } => return (token, host_id, generation),
            _ => continue,
        }
    }
}

#[test]
fn stalled_worker_returns_start_immediately_and_same_host_is_excluded() {
    let (_root, mut runtime) = fixture();
    let host_id = runtime.local_host_id();
    let barrier = Arc::new(Barrier::new(2));
    let worker_barrier = Arc::clone(&barrier);
    let (events, _receiver) = mpsc::channel();
    let started = Instant::now();

    runtime
        .start_host_operation(
            host_id,
            "stalled agent handshake",
            context(),
            events.clone(),
            Box::new(move |_| {
                worker_barrier.wait();
                while !CommandCancellation::scope_is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err("cancelled handshake".to_owned())
            }),
        )
        .unwrap();

    assert!(started.elapsed() < Duration::from_millis(250));
    assert!(runtime.host_operation_active(host_id));
    let error = runtime
        .start_host_operation(
            host_id,
            "stalled Worktrunk hook",
            context(),
            events,
            Box::new(|_| Ok(HostOperationValue::Terminated)),
        )
        .unwrap_err();
    assert!(error.contains("already busy"));
    // This is the same fast path RawInput/render uses: no wait was introduced
    // by the operation worker or the same-host exclusion check.
    assert!(started.elapsed() < Duration::from_millis(250));

    barrier.wait();
    runtime.cancel_host_operation(host_id).unwrap();
}

#[test]
fn unrelated_host_metadata_remains_available_while_local_worker_stalls() {
    let (_root, mut runtime) = fixture();
    let local_id = runtime.local_host_id();
    let remote_id = runtime.test_add_ssh_slot("other", "other.example");
    let barrier = Arc::new(Barrier::new(2));
    let worker_barrier = Arc::clone(&barrier);
    let (events, _receiver) = mpsc::channel();
    runtime
        .start_host_operation(
            local_id,
            "stalled local",
            context(),
            events,
            Box::new(move |_| {
                worker_barrier.wait();
                while !CommandCancellation::scope_is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err("cancelled".into())
            }),
        )
        .unwrap();

    assert!(runtime.hosts.contains_key(&remote_id));
    assert!(runtime.host_record(remote_id).is_ok());
    barrier.wait();
    runtime.cancel_host_operation(local_id).unwrap();
}

#[test]
fn cancellation_finishes_and_restores_host_ownership() {
    let (_root, mut runtime) = fixture();
    let host_id = runtime.local_host_id();
    let (events, receiver) = mpsc::channel();
    let token = runtime
        .start_host_operation(
            host_id,
            "agent health handshake",
            context(),
            events,
            Box::new(|_| {
                while !CommandCancellation::scope_is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err("cancelled agent health handshake".to_owned())
            }),
        )
        .unwrap();
    let (generation, current) = runtime.test_operation_identity(host_id).unwrap();
    assert_eq!(current, token);

    runtime.cancel_host_operation(host_id).unwrap();
    let (completed_token, completed_host, completed_generation) = completion(&receiver);
    assert_eq!(
        (completed_token, completed_host, completed_generation),
        (token, host_id, generation)
    );
    let completed = runtime
        .finish_host_operation(host_id, generation, token)
        .unwrap();
    assert!(matches!(completed.result, Err(error) if error.contains("cancelled")));
    assert!(!runtime.host_operation_active(host_id));
    assert!(runtime.hosts.contains_key(&host_id));
}

#[test]
fn panic_is_reported_and_host_is_restored() {
    let (_root, mut runtime) = fixture();
    let host_id = runtime.local_host_id();
    let (events, receiver) = mpsc::channel();
    let token = runtime
        .start_host_operation(
            host_id,
            "panicking hook",
            context(),
            events,
            Box::new(|_| panic!("fixture panic")),
        )
        .unwrap();
    let (_, _, generation) = completion(&receiver);
    let completed = runtime
        .finish_host_operation(host_id, generation, token)
        .unwrap();
    assert!(matches!(completed.result, Err(error) if error.contains("panicked")));
    assert!(runtime.hosts.contains_key(&host_id));
}

#[test]
fn stale_generation_cannot_consume_or_merge_current_outcome() {
    let (_root, mut runtime) = fixture();
    let host_id = runtime.local_host_id();
    let (events, receiver) = mpsc::channel();
    let token = runtime
        .start_host_operation(
            host_id,
            "one shot",
            context(),
            events,
            Box::new(|_| Ok(HostOperationValue::Terminated)),
        )
        .unwrap();
    let (_, _, generation) = completion(&receiver);

    assert!(runtime
        .finish_host_operation(host_id, generation.saturating_add(1), token)
        .is_none());
    assert!(runtime.host_operation_active(host_id));
    assert!(runtime
        .finish_host_operation(host_id, generation, token)
        .is_some());
}

#[test]
fn stale_ungroup_generation_cannot_return_authoritative_workspace() {
    let (_root, mut runtime) = fixture();
    let host_id = runtime.local_host_id();
    let mut returned = crate::core::WorkspaceRecord::new(host_id, "/tmp/ungroup-stale");
    returned.grouping = crate::core::GroupingPolicy::Ungrouped;
    let workspace_id = returned.id;
    let expected = returned.clone();
    let (events, receiver) = mpsc::channel();
    let token = runtime
        .start_host_operation(
            host_id,
            "ungroup workspace",
            HostOperationContext::WorkspaceUngroup { workspace_id },
            events,
            Box::new(move |_| Ok(HostOperationValue::WorkspaceUngrouped(returned))),
        )
        .unwrap();
    let (_, _, generation) = completion(&receiver);

    assert!(runtime
        .finish_host_operation(host_id, generation.saturating_add(1), token)
        .is_none());
    assert!(runtime.host_operation_active(host_id));
    let completed = runtime
        .finish_host_operation(host_id, generation, token)
        .unwrap();
    assert!(matches!(
        completed.result,
        Ok(HostOperationValue::WorkspaceUngrouped(workspace)) if workspace == expected
    ));
}

#[test]
fn disconnect_marks_outcome_discarded_and_never_remerges_remote_slot() {
    let (_root, mut runtime) = fixture();
    let host_id = runtime.test_add_ssh_slot("remote", "remote.example");
    let (events, receiver) = mpsc::channel();
    let token = runtime
        .start_host_operation(
            host_id,
            "remote Worktrunk mutation",
            HostOperationContext::WorktreeMutation {
                workspace_id: WorkspaceId::new(),
                command: ClientCommand::WorktreeRemove,
                replaces_forwards: false,
            },
            events,
            Box::new(|_| {
                while !CommandCancellation::scope_is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err("Unknown after disconnect; no retry".to_owned())
            }),
        )
        .unwrap();
    let (generation, _) = runtime.test_operation_identity(host_id).unwrap();
    runtime.cancel_host_operation_for_disconnect(host_id);
    let _ = completion(&receiver);

    let completed = runtime
        .finish_host_operation(host_id, generation, token)
        .unwrap();
    assert!(completed.discarded);
    assert!(!runtime.hosts.contains_key(&host_id));
    assert!(!runtime.host_operation_active(host_id));
}

#[test]
fn shutdown_cancels_cooperative_worker_and_is_bounded() {
    let (_root, mut runtime) = fixture();
    let host_id = runtime.local_host_id();
    let (events, _receiver) = mpsc::channel();
    runtime
        .start_host_operation(
            host_id,
            "stalled Worktrunk hook",
            context(),
            events,
            Box::new(|_| {
                while !CommandCancellation::scope_is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err("cancelled without retry".to_owned())
            }),
        )
        .unwrap();

    let started = Instant::now();
    drop(runtime);
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn stalled_operation_accepts_durable_actions_without_blocking_input_path() {
    let (_root, mut runtime) = fixture();
    let host_id = runtime.local_host_id();
    let workspace_id = WorkspaceId::new();
    let entered = Arc::new(Barrier::new(2));
    let worker_entered = Arc::clone(&entered);
    let (events, _receiver) = mpsc::channel();
    runtime
        .start_host_operation(
            host_id,
            "stalled agent handshake",
            context(),
            events.clone(),
            Box::new(move |_| {
                worker_entered.wait();
                while !CommandCancellation::scope_is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err("cancelled".to_owned())
            }),
        )
        .unwrap();
    entered.wait();

    let started = Instant::now();
    let queued = runtime
        .queue_durable_actions(
            host_id,
            "record detached",
            vec![DeferredHostAction::MarkDetached { workspace_id }],
            events,
        )
        .unwrap();

    assert!(started.elapsed() < Duration::from_millis(250));
    assert!(matches!(queued, DurableActionQueue::Queued { .. }));
    runtime.cancel_host_operation(host_id).unwrap();
}

#[test]
fn disconnect_warning_preserves_worktrunk_unknown_and_queued_status_risk() {
    let (_root, mut runtime) = fixture();
    let host_id = runtime.test_add_ssh_slot("remote", "remote.example");
    let workspace_id = WorkspaceId::new();
    let (events, _receiver) = mpsc::channel();
    runtime
        .start_host_operation(
            host_id,
            "remote Worktrunk mutation",
            HostOperationContext::WorktreeMutation {
                workspace_id,
                command: ClientCommand::WorktreeRemove,
                replaces_forwards: false,
            },
            events.clone(),
            Box::new(|_| {
                while !CommandCancellation::scope_is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err("Unknown after disconnect; no retry".to_owned())
            }),
        )
        .unwrap();
    runtime
        .queue_durable_actions(
            host_id,
            "Ctrl-C",
            vec![DeferredHostAction::MarkAgentsUnknown {
                workspace_id,
                run_ids: vec![crate::core::AgentRunId::new()],
            }],
            events,
        )
        .unwrap();

    let report = runtime.disconnect_host_with_restores(host_id).unwrap();
    let warning = report.warning.unwrap();

    assert!(warning.contains("Unknown after disconnect"));
    assert!(warning.contains("will not retry"));
    assert!(warning.contains(":worktree list"));
    assert!(warning.contains("may not have persisted"));
    assert!(!runtime.hosts.contains_key(&host_id));
}

#[test]
fn durable_detach_queued_during_stall_is_applied_before_operation_completion() {
    let (_root, mut runtime) = fixture();
    let host_id = runtime.local_host_id();
    let workspace = crate::core::WorkspaceRecord::new(host_id, "/tmp/durable-detach");
    runtime.registry.upsert_workspace(&workspace).unwrap();
    let session = crate::core::SessionRecord::new(
        workspace.id,
        crate::core::SessionBackend::Zellij,
        "0.44.3",
        format!("bp-{}", workspace.id),
    );
    runtime.registry.upsert_session(&session).unwrap();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let worker_entered = Arc::clone(&entered);
    let worker_release = Arc::clone(&release);
    let (events, receiver) = mpsc::channel();
    let token = runtime
        .start_host_operation(
            host_id,
            "stalled preflight",
            context(),
            events.clone(),
            Box::new(move |_| {
                worker_entered.wait();
                worker_release.wait();
                Ok(HostOperationValue::Terminated)
            }),
        )
        .unwrap();
    entered.wait();
    runtime
        .queue_durable_actions(
            host_id,
            "record detached",
            vec![DeferredHostAction::MarkDetached {
                workspace_id: workspace.id,
            }],
            events,
        )
        .unwrap();
    release.wait();
    let (_, _, generation) = completion(&receiver);

    let completed = runtime
        .finish_host_operation(host_id, generation, token)
        .unwrap();

    assert!(matches!(
        completed.deferred_results.as_slice(),
        [DeferredHostResult::Detached { result: Err(error), .. }]
            if error.contains("matching bp-host helper")
    ));
}

#[test]
fn disconnect_after_worker_drains_queue_still_delivers_durable_failure() {
    let (_root, mut runtime) = fixture();
    let host_id = runtime.test_add_ssh_slot("remote", "remote.example");
    let workspace_id = WorkspaceId::new();
    let release = Arc::new(Barrier::new(2));
    let worker_release = Arc::clone(&release);
    let (events, receiver) = mpsc::channel();
    let token = runtime
        .start_host_operation(
            host_id,
            "stalled remote preflight",
            context(),
            events.clone(),
            Box::new(move |_| {
                worker_release.wait();
                Ok(HostOperationValue::Terminated)
            }),
        )
        .unwrap();
    let (generation, _) = runtime.test_operation_identity(host_id).unwrap();
    runtime
        .queue_durable_actions(
            host_id,
            "record detached",
            vec![DeferredHostAction::MarkDetached { workspace_id }],
            events,
        )
        .unwrap();
    release.wait();
    let _ = completion(&receiver);
    assert!(runtime.host_operations[&host_id]
        .deferred_seen
        .load(Ordering::Acquire));

    let warning = runtime.disconnect_operation_warning(host_id).unwrap();
    runtime.cancel_host_operation_for_disconnect(host_id);
    let completed = runtime
        .finish_host_operation(host_id, generation, token)
        .unwrap();

    assert!(warning.contains("may not have persisted"));
    assert!(completed.discarded);
    assert!(matches!(
        completed.deferred_results.as_slice(),
        [DeferredHostResult::Detached { result: Err(_), .. }]
    ));
    assert!(!runtime.hosts.contains_key(&host_id));
}
