use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::super::input::tab_display_label;
use super::super::state::App;
use super::layout::centered_rect;
use super::output::wrap_text_lines_unbounded;

/// Render workspace selection overlay.
pub(super) fn render_overlay(app: &App, frame: &mut ratatui::Frame, area: Rect) {
    let overlay_rect = centered_rect(60, 50, area);
    frame.render_widget(Clear, overlay_rect);
    let mut lines = Vec::new();
    if let Some(message) = &app.overlay.message {
        lines.push(Line::raw(message.clone()));
    } else {
        for (idx, name) in app.overlay.items.iter().enumerate() {
            let mut label = name.clone();
            if app.active_workspace.as_deref() == Some(name.as_str()) {
                label = format!("{label} (active)");
            }
            let style = if idx == app.overlay.selected {
                Style::default().fg(Color::Black).bg(Color::White)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(Span::styled(label, style)));
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Workspaces")
        .style(Style::default().bg(Color::Black));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(Style::default().bg(Color::Black)),
        overlay_rect,
    );
}

/// Render tab selection overlay.
pub(super) fn render_tab_overlay(app: &App, frame: &mut ratatui::Frame, area: Rect) {
    let overlay_rect = centered_rect(50, 40, area);
    frame.render_widget(Clear, overlay_rect);
    let mut lines = Vec::new();
    let tabs = app
        .active_workspace
        .as_deref()
        .and_then(|workspace| app.tabs.get(workspace));
    if let Some(tabs) = tabs {
        for (idx, tab_index) in app.tab_overlay.items.iter().enumerate() {
            let mut label = tabs
                .tabs
                .get(*tab_index)
                .map(|tab| tab_display_label(app, tab))
                .unwrap_or_else(|| "tab".to_string());
            if Some(*tab_index) == Some(tabs.active) {
                label = format!("{label} (active)");
            }
            let style = if idx == app.tab_overlay.selected {
                Style::default().fg(Color::Black).bg(Color::White)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(Span::styled(label, style)));
        }
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Tabs")
        .style(Style::default().bg(Color::Black));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(Style::default().bg(Color::Black)),
        overlay_rect,
    );
}

/// Render prompt selection overlay.
pub(super) fn render_prompt_overlay(app: &App, frame: &mut ratatui::Frame, area: Rect) {
    let overlay_rect = centered_rect(50, 40, area);
    frame.render_widget(Clear, overlay_rect);
    let mut lines = Vec::new();
    if let Some(message) = &app.prompt_overlay.message {
        lines.push(Line::raw(message.clone()));
    } else {
        for (idx, item) in app.prompt_overlay.items.iter().enumerate() {
            let style = if idx == app.prompt_overlay.selected {
                Style::default().fg(Color::Black).bg(Color::White)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(Span::styled(item.clone(), style)));
        }
    }
    let title = if app.prompt_overlay.title.is_empty() {
        "Select"
    } else {
        app.prompt_overlay.title.as_str()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(Style::default().bg(Color::Black));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(Style::default().bg(Color::Black)),
        overlay_rect,
    );
}

/// Render loading indicator overlay.
pub(super) fn render_loader(frame: &mut ratatui::Frame, area: Rect, message: &str) {
    let rect = centered_rect(60, 20, area);
    frame.render_widget(Clear, rect);
    let lines = vec![
        Line::from(Span::styled(
            "Working...",
            Style::default().fg(Color::White),
        )),
        Line::raw(""),
        Line::from(Span::styled(message, Style::default().fg(Color::DarkGray))),
    ];
    let block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(Style::default().bg(Color::Black)),
        rect,
    );
}

/// Render streaming command output overlay.
pub(super) fn render_command_overlay(app: &App, frame: &mut ratatui::Frame, area: Rect) {
    let overlay_rect = centered_rect(70, 50, area);
    frame.render_widget(Clear, overlay_rect);
    let inner_width = overlay_rect.width.saturating_sub(2) as usize;
    let max_lines = overlay_rect.height.saturating_sub(2) as usize;
    let mut lines = if app.command_overlay.output.trim().is_empty() {
        vec![Line::raw("Waiting for output...")]
    } else {
        wrap_text_lines_unbounded(&app.command_overlay.output, inner_width)
    };
    if lines.len() > max_lines {
        lines = lines.split_off(lines.len() - max_lines);
    }
    let title = if app.command_overlay.title.is_empty() {
        "Running command"
    } else {
        app.command_overlay.title.as_str()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(Style::default().bg(Color::Black));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(Style::default().bg(Color::Black)),
        overlay_rect,
    );
}
