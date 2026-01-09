use crate::app::state::{App, Mode};

pub(super) fn process_terminal_output(app: &mut App, id: u64, bytes: &[u8]) {
    let active_id = app
        .active_workspace
        .as_deref()
        .and_then(|name| app.sessions.get(name))
        .map(|session| session.terminal.id());
    let mut pending_modes = None;
    for session in app.sessions.values_mut() {
        if session.terminal.id() == id {
            session.terminal.process_bytes(bytes);
            if app.mode == Mode::Work && Some(id) == active_id {
                pending_modes = Some(session.terminal.input_modes());
            }
            break;
        }
    }
    if let Some(target) = pending_modes {
        app.queue_input_mode_target(target);
    }
}
