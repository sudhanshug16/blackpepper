//! Terminal rendering to ratatui widgets.
//!
//! Converts vt100 screen state into ratatui Line/Span primitives
//! for display. Handles:
//! - Cell-by-cell styling (colors, bold, italic, etc.)
//! - Cursor display

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use vt100::{Color as VtColor, Parser};

/// Render the visible terminal area to ratatui Lines.
pub fn render_lines(parser: &Parser, rows: u16, cols: u16) -> Vec<Line<'static>> {
    let rows = rows.max(1);
    let cols = cols.max(1);
    let screen = parser.screen();
    let (cursor_row, cursor_col) = screen.cursor_position();
    let show_cursor = !screen.hide_cursor() && screen.scrollback() == 0;

    let mut lines = Vec::with_capacity(rows as usize);
    for row in 0..rows {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut current_text = String::new();
        let mut current_style = Style::default();
        let mut has_style = false;

        for col in 0..cols {
            let cell = match screen.cell(row, col) {
                Some(cell) => cell,
                None => {
                    push_span(
                        &mut spans,
                        &mut current_text,
                        &mut current_style,
                        &mut has_style,
                        Style::default(),
                        " ".to_string(),
                    );
                    continue;
                }
            };

            // Wide character continuations are skipped
            if cell.is_wide_continuation() {
                continue;
            }

            let mut style = style_for_cell(cell);

            // Cursor display
            if show_cursor && row == cursor_row && col == cursor_col {
                style = style.add_modifier(Modifier::REVERSED);
            }

            let content = if cell.has_contents() {
                cell.contents().to_string()
            } else {
                " ".to_string()
            };

            push_span(
                &mut spans,
                &mut current_text,
                &mut current_style,
                &mut has_style,
                style,
                content,
            );
        }

        if has_style {
            spans.push(Span::styled(current_text, current_style));
        } else {
            spans.push(Span::raw(String::new()));
        }

        lines.push(Line::from(spans));
    }

    lines
}

/// Convert vt100 cell attributes to ratatui Style.
fn style_for_cell(cell: &vt100::Cell) -> Style {
    let mut style = Style::default();
    style = style.fg(map_color(cell.fgcolor()));
    style = style.bg(map_color(cell.bgcolor()));

    if cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.dim() {
        style = style.add_modifier(Modifier::DIM);
    }
    if cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if cell.inverse() {
        style = style.add_modifier(Modifier::REVERSED);
    }

    style
}

/// Map vt100 colors to ratatui colors.
fn map_color(color: VtColor) -> Color {
    match color {
        VtColor::Default => Color::Reset,
        VtColor::Idx(idx) => Color::Indexed(idx),
        VtColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Helper to batch consecutive spans with the same style.
fn push_span(
    spans: &mut Vec<Span<'static>>,
    current_text: &mut String,
    current_style: &mut Style,
    has_style: &mut bool,
    style: Style,
    content: String,
) {
    if !*has_style {
        *current_style = style;
        *has_style = true;
        current_text.push_str(&content);
        return;
    }

    if *current_style == style {
        current_text.push_str(&content);
        return;
    }

    spans.push(Span::styled(std::mem::take(current_text), *current_style));
    *current_style = style;
    current_text.push_str(&content);
}
