use crate::app::state::App;

pub(super) fn process_terminal_output(app: &mut App, id: u64, bytes: &[u8]) {
    for session in app.sessions.values_mut() {
        if session.terminal.id() == id {
            session.terminal.process_bytes(bytes);
            return;
        }
    }
}
