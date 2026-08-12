mod focus;

use super::chrome;
use super::glyph::Glyphs;
use super::style::{accent_style, section_style, ui_style};
use crate::client::{ClientMode, ClientState};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

pub(super) fn render_terminal(state: &mut ClientState, frame: &mut ratatui::Frame, area: Rect) {
    if state.mode == ClientMode::Authenticate {
        focus::render_authentication(state, frame, area);
        return;
    }
    if state.mode == ClientMode::Manage {
        if state.pending_approval.is_some() {
            focus::render_approval(state, frame, area);
            return;
        }
        if state.detail.is_some() {
            focus::render_detail(state, frame, area);
            return;
        }
    }

    // Work mode hands the whole canvas to Zellij, so the label row only exists
    // in Manage, where it aligns with the HOSTS and PORTS labels beside it.
    let body = if state.mode == ClientMode::Work || area.height <= 1 {
        area
    } else {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);
        render_session_label(state, frame, rows[0]);
        rows[1]
    };
    render_session_body(state, frame, body);
}

fn render_session_label(state: &ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let separator = Glyphs::of(state).separator();
    let mut detail = vec!["zellij".to_owned()];
    if let Some(count) = state
        .active_workspace
        .and_then(|workspace| state.connected_clients.get(&workspace).copied())
    {
        detail.push(match count {
            1 => "1 client".to_owned(),
            count => format!("{count} clients"),
        });
    }
    // Tab position comes from the host, so it is absent rather than guessed
    // when the session has not been observed yet.
    if let Some((active, total)) = state
        .active_workspace
        .and_then(|workspace| state.overviews.get(&workspace))
        .and_then(|overview| overview.active_tab.zip(overview.tab_count))
    {
        detail.push(format!("tab {active}/{total}"));
    }
    let pad = chrome::pad(area.width);
    let detail = detail.join(&format!(" {separator} "));
    frame.render_widget(
        Paragraph::new(Line::styled(
            format!(
                "{pad}{}",
                chrome::right_aligned("SESSION", &detail, chrome::inner_width(area.width))
            ),
            section_style(state),
        ))
        .style(ui_style(state)),
        area,
    );
}

fn render_session_body(state: &mut ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let Some(active) = state.active_workspace else {
        state.terminal_area = Some(area);
        let body = chrome::inner(area);
        frame.render_widget(
            Paragraph::new(first_run_lines(state, body))
                .style(ui_style(state))
                .wrap(Wrap { trim: false }),
            body,
        );
        return;
    };
    let Some(terminal) = state.terminals.get_mut(&active) else {
        state.terminal_area = Some(area);
        frame.render_widget(
            Paragraph::new("Workspace is detached. Press Enter to attach.")
                .style(ui_style(state))
                .wrap(Wrap { trim: false }),
            chrome::inner(area),
        );
        return;
    };
    state.terminal_area = Some(area);
    let resize_error = terminal.resize(area.height, area.width).err();
    let lines = terminal.render(area.height, area.width);
    if let Some(error) = resize_error {
        state.set_output(format!(
            "The attached terminal could not be resized; input remains available: {error}"
        ));
    }
    frame.render_widget(Paragraph::new(lines).style(ui_style(state)), area);
}

/// The mark, what this folder is, and what you can do about it. The version
/// lives in the header's right slot, so repeating it here would spend two rows
/// on something already on screen.
fn first_run_lines(state: &ClientState, area: Rect) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if area.width >= 12 && area.height >= 9 {
        lines.extend([
            Line::styled("█", accent_style(state).add_modifier(Modifier::BOLD)),
            Line::styled("█▀▄  █▀▄", accent_style(state).add_modifier(Modifier::BOLD)),
            Line::styled("█▄▀  █▄▀", accent_style(state).add_modifier(Modifier::BOLD)),
            Line::styled("     █", accent_style(state).add_modifier(Modifier::BOLD)),
            Line::raw(""),
        ]);
    }
    let registered = state
        .selected_workspace
        .and_then(|id| state.snapshot.workspaces.iter().find(|item| item.id == id));
    let hints: Vec<(&str, String)> = match registered {
        Some(workspace) => {
            lines.push(Line::raw(format!("{} is registered.", workspace.root_path)));
            lines.push(Line::raw(""));
            vec![
                ("enter", "open this workspace".to_owned()),
                (":host add", "work on a linux ssh host".to_owned()),
                (
                    ":agent spawn",
                    format!(
                        "codex {sep} claude {sep} opencode",
                        sep = Glyphs::of(state).separator()
                    ),
                ),
            ]
        }
        None => {
            lines.push(Line::raw("No workspace is registered here."));
            lines.push(Line::raw(""));
            vec![
                (":workspace add", "register a folder".to_owned()),
                (":host add", "work on a linux ssh host".to_owned()),
            ]
        }
    };
    // One key column, so the descriptions form a single readable edge. It
    // collapses to the natural key width when the pane cannot afford 16.
    let widest = hints.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
    let column = (widest + 2).min(16);
    for (key, description) in hints {
        let padding = column.saturating_sub(key.len()).max(1);
        if column + description.chars().count() > usize::from(area.width) {
            lines.push(Line::raw(key.to_owned()));
            continue;
        }
        lines.push(Line::from(vec![
            Span::raw(format!("{key}{}", " ".repeat(padding))),
            Span::styled(description, section_style(state)),
        ]));
    }
    lines
}
