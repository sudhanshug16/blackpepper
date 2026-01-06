use std::fs::OpenOptions;
use std::io::Write;

use crossterm::event::{MouseEvent, MouseEventKind};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

use super::terminal::{mouse_event_for_terminal, terminal_cell_from_mouse};
use super::utils::format_bytes;
use super::workspace::{active_terminal_mut, active_terminal_ref};
use crate::app::state::App;

pub(super) fn handle_mouse_event(app: &mut App, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::Down(button) => {
            app.mouse_pressed = Some(button);
        }
        MouseEventKind::Up(_) => {
            app.mouse_pressed = None;
        }
        _ => {}
    }

    send_mouse_to_active_terminal(app, mouse);

    if app.mouse_debug {
        log_mouse_debug(app, mouse);
    }
}

fn send_mouse_to_active_terminal(app: &mut App, mouse: MouseEvent) -> bool {
    let Some(term_mouse) = mouse_event_for_terminal(app, mouse) else {
        return false;
    };
    let pressed = app.mouse_pressed;
    let Some(terminal) = active_terminal_mut(app) else {
        return false;
    };
    let (mode, encoding) = terminal.mouse_protocol();
    let mut term_mouse = term_mouse;
    if matches!(term_mouse.kind, MouseEventKind::Moved)
        && matches!(
            mode,
            MouseProtocolMode::ButtonMotion | MouseProtocolMode::AnyMotion
        )
    {
        if let Some(button) = pressed {
            term_mouse.kind = MouseEventKind::Drag(button);
        }
    }
    if let Some(bytes) = crate::terminal::mouse_event_to_bytes(term_mouse, mode, encoding) {
        terminal.write_bytes(&bytes);
        return true;
    }
    false
}

fn log_mouse_debug(app: &mut App, mouse: MouseEvent) {
    let in_terminal = terminal_cell_from_mouse(app, &mouse).is_some();
    let (mode, encoding, alt_screen, term_pos, encoded) = active_terminal_ref(app)
        .map(|terminal| {
            let (mode, encoding) = terminal.mouse_protocol();
            let alt = terminal.alternate_screen();
            let term_pos =
                mouse_event_for_terminal(app, mouse).map(|event| (event.row, event.column));
            let encoded = mouse_event_for_terminal(app, mouse).and_then(|event| {
                crate::terminal::mouse_event_to_bytes(event, mode, encoding)
                    .map(|bytes| format_bytes(&bytes))
            });
            (mode, encoding, alt, term_pos, encoded)
        })
        .unwrap_or((
            MouseProtocolMode::None,
            MouseProtocolEncoding::Default,
            false,
            None,
            None,
        ));
    let line = format!(
        "mouse {:?} row={} col={} in_term={} mode={:?} enc={:?} alt={} pressed={:?} term_pos={:?} bytes={}",
        mouse.kind,
        mouse.row,
        mouse.column,
        in_terminal,
        mode,
        encoding,
        alt_screen,
        app.mouse_pressed,
        term_pos,
        encoded.unwrap_or_else(|| "-".to_string())
    );
    app.set_output(line.clone());
    append_mouse_log(app, &line);
}

fn append_mouse_log(app: &App, line: &str) {
    let Some(path) = app.mouse_log_path.as_ref() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}
