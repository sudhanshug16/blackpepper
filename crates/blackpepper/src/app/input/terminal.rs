use crossterm::event::MouseEvent;

use crate::app::state::{App, CellPos};

pub(super) fn process_terminal_output(app: &mut App, id: u64, bytes: &[u8]) {
    for session in app.sessions.values_mut() {
        if session.terminal.id() == id {
            session.terminal.process_bytes(bytes);
            return;
        }
    }
}

pub(super) fn terminal_cell_from_mouse(app: &App, mouse: &MouseEvent) -> Option<CellPos> {
    let area = app.terminal_area?;
    if mouse.row < area.y
        || mouse.row >= area.y.saturating_add(area.height)
        || mouse.column < area.x
        || mouse.column >= area.x.saturating_add(area.width)
    {
        return None;
    }
    Some(CellPos {
        row: mouse.row.saturating_sub(area.y),
        col: mouse.column.saturating_sub(area.x),
    })
}

pub(super) fn mouse_event_for_terminal(app: &App, mouse: MouseEvent) -> Option<MouseEvent> {
    let pos = terminal_cell_from_mouse(app, &mouse)?;
    Some(MouseEvent {
        row: pos.row,
        column: pos.col,
        ..mouse
    })
}
