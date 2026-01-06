use arboard::Clipboard;
use crossterm::event::{KeyCode, KeyEvent, MouseEvent};

use super::workspace::{active_terminal_mut, active_terminal_ref};
use crate::app::state::{App, CellPos, SelectionState};

pub(super) fn handle_scrollback_key(app: &mut App, key: KeyEvent) -> bool {
    if !key
        .modifiers
        .contains(crossterm::event::KeyModifiers::SHIFT)
    {
        return false;
    }
    let Some(terminal) = active_terminal_mut(app) else {
        return false;
    };
    if terminal.alternate_screen() {
        return false;
    }
    let page = terminal.rows().saturating_sub(1) as isize;
    match key.code {
        KeyCode::PageUp => {
            terminal.scroll_lines(page);
            true
        }
        KeyCode::PageDown => {
            terminal.scroll_lines(-page);
            true
        }
        KeyCode::Home => {
            terminal.scroll_to_top();
            true
        }
        KeyCode::End => {
            terminal.scroll_to_bottom();
            true
        }
        _ => false,
    }
}

pub(super) fn process_terminal_output(app: &mut App, id: u64, bytes: &[u8]) {
    for tabs in app.tabs.values_mut() {
        for tab in &mut tabs.tabs {
            if tab.id == id {
                tab.terminal.process_bytes(bytes);
                return;
            }
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

// --- Selection helpers ---

pub(super) fn clear_selection(app: &mut App) {
    app.selection = SelectionState::default();
}

pub fn normalized_selection(app: &App, rows: u16, cols: u16) -> Option<(CellPos, CellPos)> {
    if !app.selection.active {
        return None;
    }
    let mut start = app.selection.start?;
    let mut end = app.selection.end.unwrap_or(start);
    start.row = start.row.min(rows.saturating_sub(1));
    end.row = end.row.min(rows.saturating_sub(1));
    start.col = start.col.min(cols.saturating_sub(1));
    end.col = end.col.min(cols.saturating_sub(1));
    if (end.row, end.col) < (start.row, start.col) {
        std::mem::swap(&mut start, &mut end);
    }
    Some((start, end))
}

pub fn selection_ranges(app: &App, rows: u16, cols: u16) -> Option<Vec<Vec<(u16, u16)>>> {
    let (start, end) = normalized_selection(app, rows, cols)?;
    let mut ranges = vec![Vec::new(); rows as usize];
    for row in start.row..=end.row {
        let row_start = if row == start.row { start.col } else { 0 };
        let row_end = if row == end.row {
            end.col
        } else {
            cols.saturating_sub(1)
        };
        let end_exclusive = row_end.saturating_add(1);
        if let Some(row_ranges) = ranges.get_mut(row as usize) {
            row_ranges.push((row_start, end_exclusive));
        }
    }
    Some(ranges)
}

pub fn active_search_range(app: &App) -> Option<(u16, u16, u16)> {
    app.search
        .matches
        .get(app.search.active_index)
        .map(|match_| (match_.row, match_.start, match_.end))
}

pub(super) fn copy_selection(app: &mut App) -> bool {
    let Some(terminal) = active_terminal_ref(app) else {
        app.set_output("No active workspace.".to_string());
        return true;
    };
    let rows = terminal.rows();
    let cols = terminal.cols();
    let Some((start, end)) = normalized_selection(app, rows, cols) else {
        app.set_output("No selection.".to_string());
        return true;
    };
    let text = terminal.contents_between(start.row, start.col, end.row, end.col);
    if text.trim().is_empty() {
        app.set_output("Selection is empty.".to_string());
        return true;
    }
    match Clipboard::new() {
        Ok(mut clipboard) => {
            if clipboard.set_text(text).is_ok() {
                app.set_output("Copied selection.".to_string());
            } else {
                app.set_output("Failed to copy selection.".to_string());
            }
        }
        Err(err) => {
            app.set_output(format!("Clipboard unavailable: {err}"));
        }
    }
    true
}
