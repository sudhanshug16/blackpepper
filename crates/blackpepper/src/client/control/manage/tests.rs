use super::*;
use crate::client::runtime::HostOperationContext;
use crate::core::RegistrySnapshot;
use ratatui::layout::Rect;
use std::sync::mpsc;
use std::time::Duration;
use termwiz::input::{MouseButtons, MouseEvent};

#[test]
fn escape_cancels_selected_host_operation_and_keeps_event_loop_responsive() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = ClientRuntime::test_fixture(root.path());
    let host_id = runtime.local_host_id();
    let (event_tx, _event_rx) = mpsc::channel();
    let mut snapshot = RegistrySnapshot::default();
    snapshot.hosts = runtime.snapshot().unwrap().hosts;
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        snapshot,
        event_tx.clone(),
    );
    state.selected_host = Some(host_id);
    let token = runtime
        .start_host_operation(
            host_id,
            "stalled agent handshake",
            HostOperationContext::Terminate {
                workspace_id: crate::core::WorkspaceId::new(),
            },
            event_tx,
            Box::new(|_| {
                while !crate::transport::CommandCancellation::scope_is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err("cancelled".to_owned())
            }),
        )
        .unwrap();
    state
        .host_operations
        .insert(host_id, (token, "stalled agent handshake".to_owned()));

    let started = std::time::Instant::now();
    handle_key(
        &mut state,
        &mut runtime,
        KeyEvent {
            key: KeyCode::Escape,
            modifiers: Modifiers::NONE,
        },
    );

    assert!(started.elapsed() < Duration::from_millis(250));
    assert!(state.host_operations[&host_id].1.starts_with("Cancelling"));
    assert!(state
        .output
        .as_deref()
        .unwrap()
        .contains("will not be retried"));
    handle_key(
        &mut state,
        &mut runtime,
        KeyEvent {
            key: KeyCode::Char('q'),
            modifiers: Modifiers::NONE,
        },
    );
    assert!(state.should_quit);
}

#[test]
fn mouse_wheel_scrolls_only_inside_the_ports_panel() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = ClientRuntime::test_fixture(root.path());
    let (event_tx, _event_rx) = mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        runtime.snapshot().unwrap(),
        event_tx,
    );
    state.ports_area = Some(Rect::new(10, 5, 20, 8));
    state.ports_scroll = 3;

    handle_mouse(
        &mut state,
        &mut runtime,
        MouseEvent {
            // termwiz mouse coordinates are one-based.
            x: 12,
            y: 7,
            mouse_buttons: MouseButtons::VERT_WHEEL | MouseButtons::WHEEL_POSITIVE,
            modifiers: Modifiers::NONE,
        },
    );
    assert_eq!(state.ports_scroll, 0);

    handle_mouse(
        &mut state,
        &mut runtime,
        MouseEvent {
            x: 12,
            y: 7,
            mouse_buttons: MouseButtons::VERT_WHEEL,
            modifiers: Modifiers::NONE,
        },
    );
    assert_eq!(state.ports_scroll, 3);

    handle_mouse(
        &mut state,
        &mut runtime,
        MouseEvent {
            x: 1,
            y: 1,
            mouse_buttons: MouseButtons::VERT_WHEEL,
            modifiers: Modifiers::NONE,
        },
    );
    assert_eq!(state.ports_scroll, 3);
}

#[test]
fn escape_dismisses_an_approval_without_starting_a_mutation() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = ClientRuntime::test_fixture(root.path());
    let (event_tx, _event_rx) = mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        runtime.snapshot().unwrap(),
        event_tx,
    );
    state.pending_approval = Some(crate::client::state::PendingWorktrunkApproval {
        workspace_id: crate::core::WorkspaceId::new(),
        command: crate::client::command::ClientCommand::WorktreeRemove,
        approval: crate::worktrunk::WorktrunkApprovalToken {
            schema: 1,
            digest: "0".repeat(64),
        },
        review: "review".to_owned(),
    });
    state.approval_scroll = 7;

    handle_key(
        &mut state,
        &mut runtime,
        KeyEvent {
            key: KeyCode::Escape,
            modifiers: Modifiers::NONE,
        },
    );

    assert!(state.pending_approval.is_none());
    assert_eq!(state.approval_scroll, 0);
    assert_eq!(
        state.output.as_deref(),
        Some("Approval dismissed; no Worktrunk mutation ran.")
    );
    assert!(state.host_operations.is_empty());
}
