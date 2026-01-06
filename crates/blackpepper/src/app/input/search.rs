use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::terminal::TerminalSession;

use crate::app::state::{App, SearchMatch};

pub(super) fn handle_search_input(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.search.active = false;
            app.search.query.clear();
            app.search.matches.clear();
            app.search.active_index = 0;
        }
        KeyCode::Enter => {
            app.search.active = false;
            if app.search.query.trim().is_empty() {
                app.search.matches.clear();
                app.search.active_index = 0;
            }
        }
        KeyCode::Backspace => {
            app.search.query.pop();
        }
        KeyCode::Char(ch) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                app.search.query.push(ch);
            }
        }
        _ => {}
    }
}

pub(super) fn search_next(app: &mut App) {
    if app.search.matches.is_empty() {
        return;
    }
    app.search.active_index = (app.search.active_index + 1) % app.search.matches.len();
}

pub(super) fn search_prev(app: &mut App) {
    if app.search.matches.is_empty() {
        return;
    }
    if app.search.active_index == 0 {
        app.search.active_index = app.search.matches.len() - 1;
    } else {
        app.search.active_index -= 1;
    }
}

/// Compute search matches for the current terminal view.
pub fn compute_search_matches(
    query: &str,
    terminal: &TerminalSession,
    rows: u16,
    cols: u16,
) -> (Vec<SearchMatch>, Vec<Vec<(u16, u16)>>) {
    let query = query.trim();
    if query.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let lines = terminal.visible_rows_text(rows, cols);
    let needle: Vec<char> = query.to_lowercase().chars().collect();
    let mut matches = Vec::new();
    let mut ranges = vec![Vec::new(); rows as usize];

    for (row_idx, line) in lines.iter().enumerate() {
        let line_chars: Vec<char> = line.to_lowercase().chars().collect();
        if needle.is_empty() || line_chars.len() < needle.len() {
            continue;
        }
        for col in 0..=line_chars.len() - needle.len() {
            if line_chars[col..col + needle.len()] == needle {
                let start = col as u16;
                let end = (col + needle.len()) as u16;
                matches.push(SearchMatch {
                    row: row_idx as u16,
                    start,
                    end,
                });
                if let Some(row_ranges) = ranges.get_mut(row_idx) {
                    row_ranges.push((start, end));
                }
            }
        }
    }

    (matches, ranges)
}
