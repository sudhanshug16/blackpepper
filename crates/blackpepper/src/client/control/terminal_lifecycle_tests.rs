use super::*;
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
