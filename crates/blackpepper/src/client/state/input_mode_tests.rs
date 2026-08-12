use super::*;

#[test]
fn authentication_clears_manage_input_modes() {
    let (event_tx, _event_rx) = std::sync::mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        RegistrySnapshot::default(),
        event_tx,
    );
    let manage = InputModes::manage_interface();
    state.input_modes_applied = manage;
    state.mode = ClientMode::Authenticate;

    state.update_input_modes();

    assert_eq!(state.input_modes_applied, InputModes::default());
    assert_eq!(
        state.pending_input_mode_bytes,
        InputModes::default().diff_bytes(&manage)
    );
}
