//! UI rendering methods.
//!
//! Handles all drawing/rendering for the TUI:
//! - Main layout (work area, separator, output, command bar)
//! - Tab bar rendering
//! - Overlays (workspace picker, tab picker, loader)
//! - Terminal content with selection/search highlighting

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::commands::{command_help_lines, command_hint_lines};
use crate::terminal::RenderOverlay;

use super::input::{
    active_search_range, active_tab_mut, active_tab_ref, compute_search_matches, selection_ranges,
    tab_display_label,
};
use super::state::{App, Mode, WorkspaceTabs, BOTTOM_HORIZONTAL_PADDING, OUTPUT_MAX_LINES};

/// Main render entry point. Called each frame by the event loop.
pub fn render(app: &mut App, frame: &mut ratatui::Frame) {
    let area = frame.area();
    let output_lines = output_lines_owned(app, area.width as usize);
    let output_height = output_lines.len() as u16;
    let separator_height = 1u16;
    let command_height = 1u16;

    // Vertical layout: work area | separator | output | command bar
    let split_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(separator_height),
            Constraint::Length(output_height),
            Constraint::Length(command_height),
        ])
        .split(area);

    render_work_area(app, frame, split_chunks[0]);
    render_separator(app, frame, split_chunks[1], area.width as usize);

    if output_height > 0 {
        let output_area = inset_horizontal(split_chunks[2], BOTTOM_HORIZONTAL_PADDING);
        let output = Paragraph::new(output_lines).style(Style::default().fg(Color::Gray));
        frame.render_widget(output, output_area);
    }

    let command_area = inset_horizontal(split_chunks[3], BOTTOM_HORIZONTAL_PADDING);
    render_command_bar(app, frame, command_area);

    // Render overlays on top if visible
    if app.overlay.visible {
        render_overlay(app, frame, area);
    }
    if app.tab_overlay.visible {
        render_tab_overlay(app, frame, area);
    }
    if app.prompt_overlay.visible {
        render_prompt_overlay(app, frame, area);
    }
    if let Some(message) = &app.loading {
        render_loader(frame, area, message);
    }
}

/// Render the main work area (tabs + terminal or welcome screen).
fn render_work_area(app: &mut App, frame: &mut ratatui::Frame, area: Rect) {
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

/// Render workspace selection overlay.
fn render_overlay(app: &App, frame: &mut ratatui::Frame, area: Rect) {
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
fn render_tab_overlay(app: &App, frame: &mut ratatui::Frame, area: Rect) {
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
fn render_prompt_overlay(app: &App, frame: &mut ratatui::Frame, area: Rect) {
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
fn render_loader(frame: &mut ratatui::Frame, area: Rect, message: &str) {
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

/// Output area lines (command hints when typing, or output message).
fn output_lines_owned(app: &App, _width: usize) -> Vec<Line<'static>> {
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

    let mut lines = Vec::new();
    for line in message.lines() {
        lines.push(Line::raw(line.to_string()));
        if lines.len() >= OUTPUT_MAX_LINES {
            break;
        }
    }
    lines
}

/// Dashed separator line.
fn dashed_line(width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let pattern = "- ";
    pattern.repeat(width / pattern.len() + 1)[..width].to_string()
}

/// Render horizontal separator.
fn render_separator(_app: &App, frame: &mut ratatui::Frame, area: Rect, width: usize) {
    let separator = Paragraph::new(Line::raw(dashed_line(width))).style(
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    );
    frame.render_widget(separator, area);
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

/// Render command bar (mode indicator or input).
fn render_command_bar(app: &App, frame: &mut ratatui::Frame, area: Rect) {
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

// --- Layout helpers ---

/// Inset a rect horizontally by padding on each side.
fn inset_horizontal(area: Rect, padding: u16) -> Rect {
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
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_width = r.width * percent_x / 100;
    let popup_height = r.height * percent_y / 100;
    let x = r.x + (r.width.saturating_sub(popup_width)) / 2;
    let y = r.y + (r.height.saturating_sub(popup_height)) / 2;
    Rect::new(x, y, popup_width, popup_height)
}
