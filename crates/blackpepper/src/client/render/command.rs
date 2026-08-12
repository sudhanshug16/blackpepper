//! The command bar and its grounded completion list.
//!
//! The list sits directly above the status row and grows downward from it, so
//! the prompt never moves while you type.

use super::glyph::Glyphs;
use super::style::{accent_style, panel_style, section_style, selected_style};
use crate::client::{completion, ClientState};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Rows the completion panel wants, excluding the status row itself.
pub(super) fn completion_rows(state: &ClientState) -> u16 {
    if !state.command_active {
        return 0;
    }
    let candidates = completion::candidates(state, command_body(state));
    if candidates.is_empty() {
        return 0;
    }
    // One row per candidate, plus the trailing constraint line.
    (candidates.len().min(6) + 1) as u16
}

pub(super) fn render_completion(state: &ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let glyphs = Glyphs::of(state);
    let candidates = completion::candidates(state, command_body(state));
    if candidates.is_empty() || area.height == 0 {
        return;
    }
    let visible = candidates.len().min(6);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(visible as u16), Constraint::Min(0)])
        .split(area);

    let width = usize::from(area.width);
    let value_column = candidates
        .iter()
        .take(visible)
        .map(|candidate| candidate.value.chars().count())
        .max()
        .unwrap_or(0)
        .min(width.saturating_sub(4))
        .max(1);

    let lines = candidates
        .iter()
        .take(visible)
        .enumerate()
        .map(|(index, candidate)| {
            let padded = format!("  {:value_column$}  ", candidate.value);
            if index == state.command_selection.min(visible.saturating_sub(1)) {
                Line::styled(format!("{padded}{}", candidate.note), selected_style(state))
            } else {
                Line::from(vec![
                    Span::raw(padded),
                    Span::styled(candidate.note.clone(), section_style(state)),
                ])
            }
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).style(panel_style(state)), rows[0]);

    let separator = glyphs.separator();
    frame.render_widget(
        Paragraph::new(Line::styled(
            format!("  tab complete {separator} enter run {separator} esc cancel"),
            section_style(state),
        ))
        .style(panel_style(state)),
        rows[1],
    );
}

/// The single status-row form of the prompt: accent colon, what you typed, the
/// caret, and the placeholder for the argument you have not written yet.
pub(super) fn command_line(state: &ClientState) -> Line<'static> {
    let body = command_body(state).to_owned();
    let mut spans = vec![
        Span::styled(":", accent_style(state)),
        Span::raw(body.clone()),
        Span::styled(" ", Style::default().add_modifier(Modifier::REVERSED)),
    ];
    if let Some(ghost) = completion::ghost(&body) {
        spans.push(Span::styled(ghost.to_owned(), section_style(state)));
    }
    Line::from(spans)
}

pub(super) fn command_body(state: &ClientState) -> &str {
    state
        .command_input
        .strip_prefix(':')
        .unwrap_or(&state.command_input)
}
