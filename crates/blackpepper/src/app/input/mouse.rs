use std::fs::OpenOptions;
use std::io::Write;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

use super::overlay::overlay_visible;
use crate::app::state::{App, Mode, SCROLL_LINES};
use super::terminal::{
    clear_selection, copy_selection, mouse_event_for_terminal, terminal_cell_from_mouse,
};
use super::utils::format_bytes;
use super::workspace::{active_terminal_mut, active_terminal_ref};

pub(super) fn handle_mouse_event(app: &mut App, mouse: MouseEvent) {
    if app.loading.is_some() {
        return;
    }
    match mouse.kind {
        MouseEventKind::Down(button) => {
            app.mouse_pressed = Some(button);
        }
        MouseEventKind::Up(_) => {
            app.mouse_pressed = None;
        }
        _ => {}
    }
    let handled = handle_mouse(app, mouse);
    if !handled && app.mode == Mode::Work && !app.command_active && !overlay_visible(app) {
        send_mouse_to_active_terminal(app, mouse);
    }
    if app.mouse_debug {
        log_mouse_debug(app, mouse);
    }
}

fn handle_mouse(app: &mut App, mouse: MouseEvent) -> bool {
    if app.loading.is_some() {
        return false;
    }
    if app.command_active || overlay_visible(app) {
        return false;
    }

    let in_terminal = terminal_cell_from_mouse(app, &mouse).is_some();
    let mouse_mode = active_terminal_ref(app)
        .map(|terminal| terminal.mouse_protocol().0)
        .unwrap_or(MouseProtocolMode::None);
    if in_terminal && mouse_mode != MouseProtocolMode::None {
        return false;
    }

    // Scrolling in terminal area (when not in mouse mode)
    if in_terminal
        && matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        )
    {
        if let Some(terminal) = active_terminal_mut(app) {
            let (mode, _) = terminal.mouse_protocol();
            if mode == MouseProtocolMode::None && !terminal.alternate_screen() {
                let delta = match mouse.kind {
                    MouseEventKind::ScrollUp => SCROLL_LINES,
                    MouseEventKind::ScrollDown => -SCROLL_LINES,
                    _ => 0,
                };
                terminal.scroll_lines(delta);
                return true;
            }
        }
    }

    // Text selection
    if in_terminal && mouse_mode == MouseProtocolMode::None {
        if let Some(pos) = terminal_cell_from_mouse(app, &mouse) {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                app.selection.active = true;
                app.selection.selecting = true;
                app.selection.start = Some(pos);
                app.selection.end = Some(pos);
                return true;
            }
            if matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left))
                && app.selection.selecting
            {
                app.selection.end = Some(pos);
                return true;
            }
            if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left))
                && app.selection.selecting
            {
                app.selection.end = Some(pos);
                app.selection.selecting = false;
                if app.selection.start != app.selection.end {
                    let copied = copy_selection(app);
                    clear_selection(app);
                    return copied;
                }
                clear_selection(app);
                return true;
            }
        }
    }

    // Tab bar clicks
    if !matches!(
        mouse.kind,
        MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
    ) {
        return false;
    }

    let in_tab_bar = app.tab_bar_area.is_some_and(|area| {
        mouse.row >= area.y
            && mouse.row < area.y.saturating_add(area.height)
            && mouse.column >= area.x
            && mouse.column < area.x.saturating_add(area.width)
    });
    if !in_tab_bar {
        return false;
    }

    let Some(workspace) = app.active_workspace.as_deref() else {
        return false;
    };
    let clicked = {
        let Some(tabs) = app.tabs.get(workspace) else {
            return false;
        };
        if tabs.tabs.is_empty() {
            return false;
        }
        let mut cursor = 0u16;
        let mut hit = None;
        for (index, tab) in tabs.tabs.iter().enumerate() {
            let label = format!(
                "{}:{}",
                index + 1,
                super::workspace::tab_display_label(app, tab)
            );
            let width = label.chars().count() as u16;
            if mouse.column >= cursor && mouse.column < cursor.saturating_add(width) {
                hit = Some(index);
                break;
            }
            cursor = cursor.saturating_add(width);
            if index + 1 < tabs.tabs.len() {
                cursor = cursor.saturating_add(2);
            }
        }
        hit
    };

    if let Some(index) = clicked {
        if let Some(tabs) = app.tabs.get_mut(workspace) {
            tabs.active = index;
        }
        if app.mouse_debug {
            app.set_output(format!(
                "Mouse click: col={} -> tab {}",
                mouse.column,
                index + 1
            ));
        }
        return true;
    }

    false
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
    let mut file = match OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => file,
        Err(_) => return,
    };
    let _ = writeln!(file, "{line}");
}
