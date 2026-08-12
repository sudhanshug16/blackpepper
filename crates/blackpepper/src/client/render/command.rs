//! The command bar and its grounded completion list.
//!
//! The list sits directly above the status row and grows downward from it, so
//! the prompt never moves while you type. It draws on the canvas rather than
//! the raised tier — the design treats it as content over the session, not as
//! another panel.

use super::chrome;
use super::glyph::Glyphs;
use super::style::{accent_style, danger_style, mid_style, section_style, ui_style};
use crate::client::state::{MouseAction, MouseTarget};
use crate::client::{completion, ClientState};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Candidate rows shown at once.
const VISIBLE: usize = 6;

/// Width of the value column. Fixed rather than content-derived so the notes
/// form one straight edge no matter which command you are part-way through.
const VALUE_COLUMN: usize = 22;

/// Rows the completion panel wants, excluding the status row itself.
pub(super) fn completion_rows(state: &ClientState) -> u16 {
    if !state.command_active {
        return 0;
    }
    let candidates = completion::candidates(state, command_body(state));
    let visible = candidates.len().min(VISIBLE);
    // Keep feedback visible even when the current input has no candidates.
    (visible + usize::from(visible > 0) + 1) as u16
}

pub(super) fn render_completion(state: &mut ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let glyphs = Glyphs::of(state);
    let candidates = completion::candidates(state, command_body(state));
    if area.height == 0 {
        return;
    }
    let pad = chrome::pad(area.width);
    let inner = chrome::inner_width(area.width);
    let visible = candidates
        .len()
        .min(VISIBLE)
        .min(usize::from(area.height.saturating_sub(1)));
    let selected = state
        .command_selection
        .map(|index| index.min(candidates.len().saturating_sub(1)));
    let offset = selected
        .unwrap_or(0)
        .saturating_sub(visible.saturating_sub(1))
        .min(candidates.len().saturating_sub(visible));
    let separator_height = u16::from(visible > 0 && area.height > visible as u16 + 1);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(visible as u16),
            Constraint::Length(separator_height),
            Constraint::Min(0),
        ])
        .split(area);

    let value_column = VALUE_COLUMN.min(inner.saturating_sub(4)).max(1);
    let lines = candidates
        .iter()
        .skip(offset)
        .take(visible)
        .enumerate()
        .map(|(row, candidate)| {
            let index = offset + row;
            let value = fit(glyphs, &candidate.value, value_column);
            let padding = value_column.saturating_sub(Line::raw(&value).width());
            // Only the value carries the selection cue, so the note column
            // stays a readable dim run rather than a reversed block.
            let value_style = if Some(index) == selected {
                ui_style(state).add_modifier(Modifier::REVERSED)
            } else {
                mid_style(state)
            };
            Line::from(vec![
                Span::raw(pad.clone()),
                Span::styled(value, value_style),
                Span::raw(" ".repeat(padding + 2)),
                Span::styled(
                    fit(
                        glyphs,
                        &candidate.note,
                        inner.saturating_sub(value_column + 2),
                    ),
                    section_style(state),
                ),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).style(ui_style(state)), rows[0]);
    frame.render_widget(Paragraph::new("").style(ui_style(state)), rows[1]);
    state
        .mouse_targets
        .extend((0..visible).map(|row| MouseTarget {
            area: Rect::new(
                rows[0].x,
                rows[0].y.saturating_add(row as u16),
                rows[0].width,
                1,
            ),
            action: MouseAction::ChooseCompletion(offset + row),
        }));

    let separator = glyphs.separator();
    let constraint = completion::constraint(command_body(state))
        .map(|constraint| format!("{constraint} {separator} "))
        .unwrap_or_default();
    let cancel = "[cancel]";
    let (hint, hint_style) = match state.command_error.as_ref() {
        Some(error) => (format!("{pad}{error}  {cancel}"), danger_style(state)),
        None => (
            format!(
                "{pad}{constraint}click/tab complete {separator} enter choose/run {separator} {cancel}"
            ),
            section_style(state),
        ),
    };
    let hint = fit(glyphs, &hint, usize::from(area.width));
    let cancel_x = hint
        .find(cancel)
        .map(|index| Line::raw(&hint[..index]).width());
    frame.render_widget(
        Paragraph::new(Line::styled(hint, hint_style)).style(ui_style(state)),
        rows[2],
    );
    if let Some(x) = cancel_x.filter(|x| *x < usize::from(rows[2].width)) {
        state.mouse_targets.push(MouseTarget {
            area: Rect::new(
                rows[2].x.saturating_add(x as u16),
                rows[2].y,
                Line::raw(cancel)
                    .width()
                    .min(usize::from(rows[2].width) - x) as u16,
                1,
            ),
            action: MouseAction::CloseCommand,
        });
    }
}

/// The single status-row form of the prompt: accent colon, what you typed, the
/// placeholder for the argument still missing, and the caret at the end.
pub(super) fn command_line(state: &ClientState, width: u16) -> Line<'static> {
    let body = command_body(state).to_owned();
    let mut spans = vec![
        Span::styled(":", accent_style(state)),
        Span::raw(body.clone()),
    ];
    // The caret is the one thing that must never be dropped — it is where the
    // next keystroke lands. The placeholder yields to it.
    if let Some(ghost) = completion::ghost(&body) {
        let used = 2 * usize::from(chrome::gutter(width)) + 10 + body.chars().count() + 2;
        if used + ghost.chars().count() <= usize::from(width) {
            spans.push(Span::styled(ghost.to_owned(), section_style(state)));
        }
    }
    spans.push(Span::styled(
        " ",
        Style::default().add_modifier(Modifier::REVERSED),
    ));
    Line::from(spans)
}

pub(super) fn command_body(state: &ClientState) -> &str {
    state
        .command_input
        .strip_prefix(':')
        .unwrap_or(&state.command_input)
}

fn fit(glyphs: Glyphs, value: &str, columns: usize) -> String {
    if Line::raw(value).width() <= columns {
        return value.to_owned();
    }
    if columns == 0 {
        return String::new();
    }
    let ellipsis = glyphs.ellipsis();
    let mut output = String::new();
    for character in value.chars() {
        let candidate = format!("{output}{character}{ellipsis}");
        if Line::raw(&candidate).width() > columns {
            break;
        }
        output.push(character);
    }
    output.push_str(ellipsis);
    output
}
