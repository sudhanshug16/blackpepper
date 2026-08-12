use super::*;
use crate::client::runtime::HostOperationContext;
use crate::core::RegistrySnapshot;
use crate::transport::{ProcessSpec, PtyProcess};
use portable_pty::PtySize;
use ratatui::layout::Rect;
use ratatui::{backend::TestBackend, Terminal};
use std::sync::mpsc;
use std::time::Duration;
use termwiz::input::{MouseButtons, MouseEvent};

fn attached_fixture() -> (tempfile::TempDir, ClientRuntime, ClientState) {
    let root = tempfile::tempdir().unwrap();
    let workspace_root = root.path().join("workspace");
    std::fs::create_dir(&workspace_root).unwrap();
    let mut runtime = ClientRuntime::test_fixture(root.path());
    let host_id = runtime.local_host_id();
    let workspace_id = runtime
        .register_workspace(host_id, &workspace_root)
        .unwrap();
    let (event_tx, _event_rx) = mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        runtime.snapshot().unwrap(),
        event_tx,
    );
    state.selected_workspace = Some(workspace_id);
    let process = PtyProcess::spawn(
        &ProcessSpec::new("sh").args(["-c", "exec sleep 30"]),
        PtySize::default(),
    )
    .unwrap();
    apply_attachment(&mut state, workspace_id, process, 1).unwrap();
    (root, runtime, state)
}

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
    state.mouse_targets.push(crate::client::state::MouseTarget {
        area: Rect::new(10, 5, 20, 8),
        action: crate::client::state::MouseAction::ScrollPorts,
    });
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

#[test]
fn clicking_the_manage_session_enters_work_mode() {
    let (_root, mut runtime, mut state) = attached_fixture();
    state.mode = ClientMode::Manage;
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal
        .draw(|frame| crate::client::render(&mut state, frame))
        .unwrap();
    let area = state.terminal_area.expect("rendered session area");

    handle_mouse(
        &mut state,
        &mut runtime,
        MouseEvent {
            x: area.x + 1,
            y: area.y + 1,
            mouse_buttons: MouseButtons::LEFT,
            modifiers: Modifiers::NONE,
        },
    );

    assert_eq!(state.mode, ClientMode::Work);
}

#[test]
fn clicking_the_work_footer_enters_manage_without_forwarding_the_click() {
    let (_root, mut runtime, mut state) = attached_fixture();
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal
        .draw(|frame| crate::client::render(&mut state, frame))
        .unwrap();

    super::super::handle_raw(&mut state, &mut runtime, b"\x1b[<0;5;24M");

    assert_eq!(state.mode, ClientMode::Manage);
}

#[test]
fn clicking_a_command_candidate_advances_to_its_first_argument() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = ClientRuntime::test_fixture(root.path());
    let (event_tx, _event_rx) = mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        runtime.snapshot().unwrap(),
        event_tx,
    );
    state.command_active = true;
    state.command_input = ":host".to_owned();
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal
        .draw(|frame| crate::client::render(&mut state, frame))
        .unwrap();
    let target = state
        .mouse_targets
        .iter()
        .find(|target| {
            matches!(
                target.action,
                crate::client::state::MouseAction::ChooseCompletion(0)
            )
        })
        .expect("first command candidate")
        .area;

    handle_mouse(
        &mut state,
        &mut runtime,
        MouseEvent {
            x: target.x + 1,
            y: target.y + 1,
            mouse_buttons: MouseButtons::LEFT,
            modifiers: Modifiers::NONE,
        },
    );

    assert_eq!(state.command_input, ":host add ");
}

#[test]
fn incomplete_command_stays_open_with_specific_usage() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = ClientRuntime::test_fixture(root.path());
    let (event_tx, _event_rx) = mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        runtime.snapshot().unwrap(),
        event_tx,
    );
    state.command_active = true;
    state.command_input = ":host add ".to_owned();

    handle_key(
        &mut state,
        &mut runtime,
        KeyEvent {
            key: KeyCode::Enter,
            modifiers: Modifiers::NONE,
        },
    );

    assert!(state.command_active);
    assert_eq!(state.command_input, ":host add ");
    assert!(state
        .command_error
        .as_deref()
        .is_some_and(|error| error.starts_with("Usage: :host add")));
}

/// Arrow keys then Enter run the highlighted candidate, while typing or
/// stepping above the first row returns to the exact text on screen.
#[cfg(test)]
mod completion_selection {
    use super::*;
    use termwiz::input::{KeyCode, KeyEvent, Modifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            key: code,
            modifiers: Modifiers::NONE,
        }
    }

    fn fixture() -> (ClientState, ClientRuntime) {
        let root = tempfile::tempdir().unwrap();
        let runtime = ClientRuntime::test_fixture(root.path());
        let (event_tx, _event_rx) = mpsc::channel();
        let state = ClientState::new(
            crate::client_config::load_contents(None, None, None).unwrap(),
            RegistrySnapshot {
                hosts: runtime.snapshot().unwrap().hosts,
                ..RegistrySnapshot::default()
            },
            event_tx,
        );
        std::mem::forget(root);
        (state, runtime)
    }

    fn typed(state: &mut ClientState, runtime: &mut ClientRuntime, text: &str) {
        state.command_active = true;
        state.command_input = ":".to_owned();
        for character in text.chars() {
            super::handle_key(state, runtime, key(KeyCode::Char(character)));
        }
    }

    #[test]
    fn enter_runs_the_highlighted_candidate_not_the_typed_prefix() {
        let (mut state, mut runtime) = fixture();
        typed(&mut state, &mut runtime, "the");
        super::handle_key(&mut state, &mut runtime, key(KeyCode::DownArrow));
        assert_eq!(state.command_selection, Some(0));
        super::handle_key(&mut state, &mut runtime, key(KeyCode::Enter));

        assert!(!state.command_active);
        let output = state.output.clone().unwrap_or_default();
        assert!(!output.contains("begin with ':'") && !output.contains("Unknown"));
    }

    #[test]
    fn completing_a_command_that_wants_an_argument_keeps_the_bar_open() {
        let (mut state, mut runtime) = fixture();
        typed(&mut state, &mut runtime, "hos");
        super::handle_key(&mut state, &mut runtime, key(KeyCode::DownArrow));
        super::handle_key(&mut state, &mut runtime, key(KeyCode::Enter));
        assert!(state.command_active);
        assert!(state.command_input.ends_with(' '));
    }

    #[test]
    fn arrowing_back_off_the_list_returns_to_what_was_typed() {
        let (mut state, mut runtime) = fixture();
        typed(&mut state, &mut runtime, "the");
        super::handle_key(&mut state, &mut runtime, key(KeyCode::DownArrow));
        super::handle_key(&mut state, &mut runtime, key(KeyCode::UpArrow));
        assert_eq!(state.command_selection, None);
        assert_eq!(state.command_input, ":the");
    }

    #[test]
    fn typing_clears_the_highlight_so_enter_runs_what_is_on_screen() {
        let (mut state, mut runtime) = fixture();
        typed(&mut state, &mut runtime, "the");
        super::handle_key(&mut state, &mut runtime, key(KeyCode::DownArrow));
        super::handle_key(&mut state, &mut runtime, key(KeyCode::Char('m')));
        assert_eq!(state.command_selection, None);
    }
}
