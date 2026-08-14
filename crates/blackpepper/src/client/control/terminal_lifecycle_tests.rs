use super::*;
use crate::client::EmbeddedTerminal;
use crate::transport::{ProcessSpec, PtyProcess};
use portable_pty::PtySize;
use std::sync::mpsc;

#[test]
fn closed_terminal_input_requests_clean_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = ClientRuntime::test_fixture(root.path());
    let (event_tx, _event_rx) = mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        runtime.snapshot().unwrap(),
        event_tx,
    );

    handle_event(&mut state, &mut runtime, ClientEvent::TerminalInputClosed);

    assert!(state.should_quit);
}

fn focus_terminal(
    workspace_id: crate::core::WorkspaceId,
    event_tx: mpsc::Sender<ClientEvent>,
    reporting: bool,
) -> EmbeddedTerminal {
    let process = PtyProcess::spawn(
        &ProcessSpec::new("sh").args(["-c", "exec sleep 30"]),
        PtySize::default(),
    )
    .unwrap();
    let mut terminal = EmbeddedTerminal::new(
        workspace_id,
        process,
        24,
        80,
        (255, 255, 255),
        (0, 0, 0),
        event_tx,
    )
    .unwrap();
    if reporting {
        terminal.process_bytes(b"\x1b[?1004h");
    }
    terminal
}

fn focus_fixture(
    first_reporting: bool,
    second_reporting: bool,
) -> (
    tempfile::TempDir,
    ClientRuntime,
    ClientState,
    crate::core::WorkspaceId,
    crate::core::WorkspaceId,
) {
    let root = tempfile::tempdir().unwrap();
    let first_root = root.path().join("first");
    let second_root = root.path().join("second");
    std::fs::create_dir(&first_root).unwrap();
    std::fs::create_dir(&second_root).unwrap();
    let mut runtime = ClientRuntime::test_fixture(root.path());
    let host_id = runtime.local_host_id();
    let first = runtime.register_workspace(host_id, &first_root).unwrap();
    let second = runtime.register_workspace(host_id, &second_root).unwrap();
    let (event_tx, _event_rx) = mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        runtime.snapshot().unwrap(),
        event_tx.clone(),
    );
    state.terminals.insert(
        first,
        focus_terminal(first, event_tx.clone(), first_reporting),
    );
    state
        .terminals
        .insert(second, focus_terminal(second, event_tx, second_reporting));
    state.selected_workspace = Some(first);
    state.active_workspace = Some(first);
    state.mode = ClientMode::Work;
    (root, runtime, state, first, second)
}

#[test]
fn manage_toggle_sends_visibility_focus_out_and_back_in() {
    let (_root, mut runtime, mut state, first, _second) = focus_fixture(true, false);
    state.update_input_modes();
    assert_eq!(
        state.terminals[&first].visibility_focus_history_for_test(),
        [true]
    );

    handle_matched_chord(&mut state, &mut runtime, MatchedChord::Toggle);
    state.update_input_modes();
    assert_eq!(state.mode, ClientMode::Manage);
    assert_eq!(
        state.terminals[&first].visibility_focus_history_for_test(),
        [true, false]
    );

    handle_matched_chord(&mut state, &mut runtime, MatchedChord::Toggle);
    state.update_input_modes();
    assert_eq!(state.mode, ClientMode::Work);
    assert_eq!(
        state.terminals[&first].visibility_focus_history_for_test(),
        [true, false, true]
    );
}

#[test]
fn workspace_switch_moves_visibility_focus_from_first_to_second() {
    let (_root, mut runtime, mut state, first, second) = focus_fixture(true, true);
    state.update_input_modes();
    assert_eq!(
        state.terminals[&first].visibility_focus_history_for_test(),
        [true]
    );
    assert_eq!(
        state.terminals[&second].visibility_focus_history_for_test(),
        [false]
    );

    state.selected_workspace = Some(second);
    attach_selected(&mut state, &mut runtime);
    state.update_input_modes();

    assert_eq!(state.active_workspace, Some(second));
    assert_eq!(
        state.terminals[&first].visibility_focus_history_for_test(),
        [true, false]
    );
    assert_eq!(
        state.terminals[&second].visibility_focus_history_for_test(),
        [false, true]
    );
}

#[test]
fn visibility_focus_handoff_requires_dec_mode_1004() {
    let (_root, mut runtime, mut state, first, _second) = focus_fixture(false, false);
    state.update_input_modes();
    handle_matched_chord(&mut state, &mut runtime, MatchedChord::Toggle);
    state.update_input_modes();
    handle_matched_chord(&mut state, &mut runtime, MatchedChord::Toggle);
    state.update_input_modes();

    assert!(state.terminals[&first]
        .visibility_focus_history_for_test()
        .is_empty());
}

#[test]
fn workspace_switch_does_not_invent_focus_while_outer_window_is_unfocused() {
    let (_root, mut runtime, mut state, first, second) = focus_fixture(true, true);
    state.update_input_modes();

    handle_event(
        &mut state,
        &mut runtime,
        ClientEvent::RawInput(b"\x1b[O".to_vec()),
    );
    assert_eq!(
        state.terminals[&first].visibility_focus_history_for_test(),
        [true, false]
    );

    state.selected_workspace = Some(second);
    attach_selected(&mut state, &mut runtime);
    state.update_input_modes();
    assert_eq!(
        state.terminals[&second].visibility_focus_history_for_test(),
        [false]
    );

    handle_event(
        &mut state,
        &mut runtime,
        ClientEvent::RawInput(b"\x1b[I".to_vec()),
    );
    assert_eq!(
        state.terminals[&second].visibility_focus_history_for_test(),
        [false, true]
    );
}
