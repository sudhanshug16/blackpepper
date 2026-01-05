//! Reusable widget rendering functions.
//!
//! Pure functions that produce ratatui widgets from data.
//! No state mutation happens here.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::layout::centered_rect;

/// Render a dashed separator line.
pub fn render_separator(frame: &mut Frame, area: Rect, width: usize) {
    let pattern = "- ";
    let line = if width == 0 {
        String::new()
    } else {
        pattern.repeat(width / pattern.len() + 1)[..width].to_string()
    };
    let separator = Paragraph::new(Line::raw(line)).style(
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    );
    frame.render_widget(separator, area);
}

/// Render a centered loading overlay.
pub fn render_loader(frame: &mut Frame, area: Rect, message: &str) {
    let rect = centered_rect(60, 20, area);
    let lines = vec![
        Line::from(Span::styled(
            "Working...",
            Style::default().fg(Color::White),
        )),
        Line::raw(""),
        Line::from(Span::styled(message, Style::default().fg(Color::DarkGray))),
    ];
    frame.render_widget(Paragraph::new(lines), rect);
}

/// Render a list overlay (for workspace/tab selection).
pub fn render_overlay(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    items: &[String],
    selected: usize,
    active_marker: Option<&str>,
    message: Option<&str>,
) {
    let overlay_rect = centered_rect(60, 50, area);
    let mut lines = Vec::new();

    if let Some(msg) = message {
        lines.push(Line::raw(msg.to_string()));
    } else {
        for (idx, name) in items.iter().enumerate() {
            let mut label = name.clone();
            if active_marker == Some(name.as_str()) {
                label = format!("{label} (active)");
            }
            let style = if idx == selected {
                Style::default().fg(Color::Black).bg(Color::White)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(Span::styled(label, style)));
        }
    }

    let block = Block::default().borders(Borders::ALL).title(title);
    frame.render_widget(Paragraph::new(lines).block(block), overlay_rect);
}
