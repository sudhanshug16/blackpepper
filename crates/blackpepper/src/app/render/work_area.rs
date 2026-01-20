use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::commands::command_help_lines;

use super::super::state::App;

/// Render the main work area (terminal or welcome screen).
pub(super) fn render_work_area(app: &mut App, frame: &mut ratatui::Frame, area: Rect) {
    let Some(workspace) = app.active_workspace.as_deref() else {
        app.terminal_area = None;
        let body_lines = body_lines(app);
        frame.render_widget(Paragraph::new(body_lines), area);
        return;
    };

    let Some(session) = app.sessions.get_mut(workspace) else {
        app.terminal_area = None;
        let body_lines = body_lines(app);
        frame.render_widget(Paragraph::new(body_lines), area);
        return;
    };

    app.terminal_area = Some(area);
    let terminal = &mut session.terminal;
    terminal.resize(area.height, area.width);
    let lines = terminal.render_lines(area.height, area.width);
    frame.render_widget(Paragraph::new(lines), area);
}

/// Welcome/no-workspace body text.
fn body_lines(app: &App) -> Vec<Line<'_>> {
    // ASCII art logo
    let mut lines = vec![
        Line::from(Span::styled(
            "                _     _  _       ",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            "|_  |  _  _  | |_) _ |_)|_) _  __",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            "|_) | (_|(_  |<|  (/_|  |  (/_ | ",
            Style::default().fg(Color::Yellow),
        )),
        Line::raw(""),
    ];
    if let Some(workspace) = &app.active_workspace {
        lines.push(Line::raw(format!("Active workspace: {workspace}")));
        lines.push(Line::raw(format!("Path: {}", app.cwd.to_string_lossy())));
    } else {
        let toggle_mode = keymap_label(&app.config.keymap.toggle_mode);
        let switch_workspace = keymap_label(&app.config.keymap.switch_workspace);
        lines.push(Line::raw("No active workspace."));
        lines.push(Line::raw("Quick start:"));
        lines.push(Line::raw(
            "- Modes: Manage (app commands/navigation), Work (keys to tmux).",
        ));
        lines.push(Line::raw(format!("- Toggle mode: {toggle_mode}")));
        lines.push(Line::raw("- Work mode requires an active workspace."));
        lines.push(Line::raw("- Open command bar (Manage): :"));
        lines.push(Line::raw(format!(
            "- Cycle workspace: {switch_workspace} or :workspace list"
        )));
        lines.push(Line::raw("- Create workspace: :workspace create [name]"));
        lines.push(Line::raw("- Quit: q (Manage) or :quit"));
        lines.push(Line::raw(""));
        lines.push(Line::raw("Commands:"));
        for command in command_help_lines() {
            lines.push(Line::raw(command));
        }
    }
    lines
}

fn keymap_label(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "unbound".to_string()
    } else {
        trimmed.to_string()
    }
}
