use ratatui::text::Line;

use crate::commands::command_hint_lines;

use super::super::state::{App, OUTPUT_MAX_LINES};

/// Output area lines (command hints when typing, or output message).
pub(super) fn output_lines_owned(app: &App, width: usize) -> Vec<Line<'static>> {
    if app.command_active {
        let hints = command_hint_lines(&app.command_input, OUTPUT_MAX_LINES);
        let mut lines = Vec::new();
        if hints.is_empty() {
            lines.push(Line::raw("No commands found."));
        } else {
            for line in hints {
                lines.push(Line::raw(line));
                if lines.len() >= OUTPUT_MAX_LINES {
                    break;
                }
            }
        }
        return lines;
    }

    let Some(message) = app.output.as_ref() else {
        return Vec::new();
    };
    if message.trim().is_empty() {
        return Vec::new();
    }

    wrap_text_lines(message, width, OUTPUT_MAX_LINES)
}

fn wrap_text_lines(message: &str, width: usize, max_lines: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for raw_line in message.lines() {
        let wrapped = wrap_preserve(raw_line, width);
        for item in wrapped {
            lines.push(Line::raw(item));
            if lines.len() >= max_lines {
                return lines;
            }
        }
        if lines.len() >= max_lines {
            return lines;
        }
    }
    lines
}

pub(super) fn wrap_text_lines_unbounded(message: &str, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for raw_line in message.lines() {
        let wrapped = wrap_preserve(raw_line, width);
        for item in wrapped {
            lines.push(Line::raw(item));
        }
    }
    lines
}

fn wrap_preserve(line: &str, width: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    let mut output = Vec::new();
    let mut current = String::new();
    let mut count = 0usize;
    for ch in line.chars() {
        if count >= width {
            output.push(current);
            current = String::new();
            count = 0;
        }
        current.push(ch);
        count += 1;
    }
    output.push(current);
    output
}
