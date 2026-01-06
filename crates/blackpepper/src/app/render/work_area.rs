use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::commands::command_help_lines;
use crate::terminal::RenderOverlay;

use super::super::input::{
    active_search_range, active_tab_mut, active_tab_ref, compute_search_matches, selection_ranges,
    tab_display_label,
};
use super::super::state::{App, WorkspaceTabs};

/// Render the main work area (tabs + terminal or welcome screen).
pub(super) fn render_work_area(app: &mut App, frame: &mut ratatui::Frame, area: Rect) {
    let Some(workspace) = app.active_workspace.as_deref() else {
        // No workspace: show welcome screen
        app.tab_bar_area = None;
        app.terminal_area = None;
        let body_lines = body_lines(app);
        frame.render_widget(Paragraph::new(body_lines), area);
        return;
    };

    let Some(tabs) = app.tabs.get(workspace) else {
        app.tab_bar_area = None;
        app.terminal_area = None;
        let body_lines = body_lines(app);
        frame.render_widget(Paragraph::new(body_lines), area);
        return;
    };

    if tabs.tabs.is_empty() {
        app.tab_bar_area = None;
        app.terminal_area = None;
        let body_lines = body_lines(app);
        frame.render_widget(Paragraph::new(body_lines), area);
        return;
    }

    // Split: tab bar | terminal
    let tab_height = 1u16;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(tab_height), Constraint::Min(1)])
        .split(area);

    render_tab_bar(app, frame, chunks[0], tabs);
    app.tab_bar_area = Some(chunks[0]);
    app.terminal_area = Some(chunks[1]);

    // Compute selection and search highlights before rendering terminal
    let (selection_ranges, search_ranges, active_search) = {
        let selection_ranges = selection_ranges(app, chunks[1].height, chunks[1].width);
        let (matches, ranges) = if let Some(tab) = active_tab_ref(app) {
            compute_search_matches(
                &app.search.query,
                &tab.terminal,
                chunks[1].height,
                chunks[1].width,
            )
        } else {
            (Vec::new(), Vec::new())
        };
        app.search.matches = matches;
        if app.search.active_index >= app.search.matches.len() {
            app.search.active_index = 0;
        }
        let active = active_search_range(app);
        let ranges = if app.search.matches.is_empty() {
            None
        } else {
            Some(ranges)
        };
        (selection_ranges, ranges, active)
    };

    // Render terminal content with overlays
    if let Some(tab) = active_tab_mut(app) {
        let terminal = &mut tab.terminal;
        terminal.resize(chunks[1].height, chunks[1].width);
        let lines = terminal.render_lines_with_overlay(
            chunks[1].height,
            chunks[1].width,
            RenderOverlay {
                selection: selection_ranges.as_deref(),
                search: search_ranges.as_deref(),
                active_search,
            },
        );
        frame.render_widget(Paragraph::new(lines), chunks[1]);
    }
}

/// Render tab bar.
fn render_tab_bar(app: &App, frame: &mut ratatui::Frame, area: Rect, tabs: &WorkspaceTabs) {
    let mut spans = Vec::new();
    for (index, tab) in tabs.tabs.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        let label = format!("{}:{}", index + 1, tab_display_label(app, tab));
        let style = if index == tabs.active {
            Style::default().fg(Color::Black).bg(Color::White)
        } else {
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM)
        };
        spans.push(Span::styled(label, style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
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
        let switch_tab = keymap_label(&app.config.keymap.switch_tab);
        let refresh = keymap_label(&app.config.keymap.refresh);

        lines.push(Line::raw("No active workspace."));
        lines.push(Line::raw("Quick start:"));
        lines.push(Line::raw(
            "- Modes: Manage (app commands/navigation), Work (keys to terminal).",
        ));
        lines.push(Line::raw(format!("- Toggle mode: {toggle_mode}")));
        lines.push(Line::raw("- Work mode requires an active workspace."));
        lines.push(Line::raw("- Open command bar (Manage): :"));
        lines.push(Line::raw(format!(
            "- Switch workspace: {switch_workspace} or :workspace list"
        )));
        lines.push(Line::raw("- Create workspace: :workspace create [name]"));
        lines.push(Line::raw(format!("- Switch tab: {switch_tab}")));
        lines.push(Line::raw(format!(
            "- Refresh UI (Manage): {refresh} or :refresh"
        )));
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
