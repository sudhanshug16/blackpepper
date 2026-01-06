use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::super::state::{App, Mode};

/// Render horizontal separator.
pub(super) fn render_separator(_app: &App, frame: &mut ratatui::Frame, area: Rect, width: usize) {
    let separator = Paragraph::new(Line::raw(dashed_line(width))).style(
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    );
    frame.render_widget(separator, area);
}

/// Render command bar (mode indicator or input).
pub(super) fn render_command_bar(app: &App, frame: &mut ratatui::Frame, area: Rect) {
    let workspace_name = app
        .active_workspace
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    if let Some(name) = workspace_name {
        let version = env!("CARGO_PKG_VERSION");
        let label_text = format!("Active workspace: {name} | v{version}");
        let width = area.width as usize;
        let label_len = label_text.chars().count();
        if width > label_len + 1 {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Min(1),
                    Constraint::Length((label_len + 1) as u16),
                ])
                .split(area);

            let command_line = command_line(app);
            frame.render_widget(Paragraph::new(command_line), chunks[0]);

            let dim_style = Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM);
            let label = Paragraph::new(Line::from(vec![
                Span::styled("Active workspace: ", dim_style),
                Span::styled(name.to_string(), dim_style),
                Span::styled(" | ", dim_style),
                Span::styled(format!("v{version}"), dim_style),
            ]))
            .alignment(Alignment::Right);
            frame.render_widget(label, chunks[1]);
            return;
        }
    }

    let command_line = command_line(app);
    frame.render_widget(Paragraph::new(command_line), area);
}

/// Build the command line content (mode label or input).
fn command_line(app: &App) -> Line<'_> {
    if app.command_active {
        // Command input mode: show input with cursor
        Line::from(vec![
            Span::styled(app.command_input.clone(), Style::default().fg(Color::White)),
            Span::styled(" ", Style::default().bg(Color::White).fg(Color::Black)),
        ])
    } else if app.search.active {
        // Search input mode
        Line::from(vec![
            Span::styled(
                format!("/{}", app.search.query),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(" ", Style::default().bg(Color::Yellow).fg(Color::Black)),
        ])
    } else {
        // Mode indicator
        let label = match app.mode {
            Mode::Work => "-- WORK --",
            Mode::Manage => "-- MANAGE --",
        };
        let style = if app.mode == Mode::Manage {
            Style::default().bg(Color::Magenta).fg(Color::Black)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        Line::from(vec![Span::styled(label, style)])
    }
}

/// Dashed separator line.
fn dashed_line(width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let pattern = "- ";
    pattern.repeat(width / pattern.len() + 1)[..width].to_string()
}

/// Inset a rect horizontally by padding on each side.
pub(super) fn inset_horizontal(area: Rect, padding: u16) -> Rect {
    if area.width <= padding * 2 {
        return area;
    }
    Rect {
        x: area.x + padding,
        width: area.width - padding * 2,
        ..area
    }
}

/// Create a centered rect with given percentage of parent.
pub(super) fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_width = r.width * percent_x / 100;
    let popup_height = r.height * percent_y / 100;
    let x = r.x + (r.width.saturating_sub(popup_width)) / 2;
    let y = r.y + (r.height.saturating_sub(popup_height)) / 2;
    Rect::new(x, y, popup_width, popup_height)
}
