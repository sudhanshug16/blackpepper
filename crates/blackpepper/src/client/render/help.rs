//! Grouped `:help`.
//!
//! Commands are grouped by what they act on, current context first, and every
//! entry carries a right-hand column describing the state it would act against.
//! A command that cannot run right now stays listed and goes dim with the
//! reason in that column — the list never lies about what will run.

use super::chord::chord_label;
use super::chrome;
use super::glyph::Glyphs;
use super::style::{accent_style, section_style, ui_style};
use crate::client::catalog::{entries, CommandGroup};
use crate::client::state::{MouseAction, MouseTarget};
use crate::client::ClientState;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Minimum gap between the syntax column and the note column.
const COLUMN_GAP: usize = 2;

pub(super) fn render_help(state: &mut ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let glyphs = Glyphs::of(state);
    // Clamp rather than let the view scroll past its own end, which would
    // otherwise leave an empty pane and no obvious way back.
    let visible = usize::from(area.height).saturating_sub(1);
    let max_scroll = help_rows(state).saturating_sub(visible);
    let scroll = state
        .help
        .map(|help| usize::from(help.scroll).min(max_scroll) as u16)
        .unwrap_or_default();
    let body = chrome::inner(area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(body);

    let separator = glyphs.separator();
    let hint = format!("esc close {separator} {} scroll", glyphs.updown());
    // Measure the rendered string, not its byte length — the glyph set changes
    // how many columns it occupies.
    let hint_pad = usize::from(rows[0].width)
        .saturating_sub(5 + Line::raw(&hint).width())
        .max(2);
    let heading = Line::from(vec![
        Span::styled(":", accent_style(state)),
        Span::raw("help"),
        Span::raw(" ".repeat(hint_pad)),
        Span::styled(hint, section_style(state)),
    ]);
    frame.render_widget(Paragraph::new(heading).style(ui_style(state)), rows[0]);

    let catalog = entries(state);
    // One column derived from the real catalog. Hardcoding the design's 26
    // would glue the longest entries to their notes.
    let syntax_column = catalog
        .iter()
        .map(|entry| entry.syntax.chars().count())
        .chain(
            [
                &state.config.keymap.toggle_mode,
                &state.config.keymap.switch_workspace,
                &state.config.keymap.workspace_overlay,
            ]
            .into_iter()
            .map(|binding| chord_label(binding).chars().count()),
        )
        .max()
        .unwrap_or(0);
    // One blank row under the heading, then one between groups — never two.
    let mut lines = vec![Line::raw("")];
    let mut row_actions = Vec::new();
    let mut emitted_group = false;
    for group in CommandGroup::ORDER {
        let group_entries = catalog
            .iter()
            .filter(|entry| entry.group == group)
            .collect::<Vec<_>>();
        if group_entries.is_empty() {
            continue;
        }
        if emitted_group {
            lines.push(Line::raw(""));
        }
        emitted_group = true;
        lines.push(Line::styled(group.heading(), section_style(state)));
        for entry in group_entries {
            let line_index = lines.len();
            let padding = syntax_column.saturating_sub(entry.syntax.chars().count());
            let syntax_style = if entry.available {
                Style::default()
            } else {
                section_style(state)
            };
            lines.push(Line::from(vec![
                Span::styled(entry.syntax, syntax_style),
                Span::raw(" ".repeat(padding + COLUMN_GAP)),
                Span::styled(entry.note.clone(), section_style(state)),
            ]));
            if entry.available {
                row_actions.push((
                    line_index,
                    MouseAction::PrefillCommand(crate::client::completion::prefill_from_syntax(
                        entry.syntax,
                    )),
                ));
            }
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("KEYS", section_style(state)));
    for (chord, meaning, action) in [
        (
            &state.config.keymap.toggle_mode,
            "switch work / manage",
            state.active_workspace.map(|_| MouseAction::EnterWork),
        ),
        (
            &state.config.keymap.switch_workspace,
            "attach next workspace",
            (!state.snapshot.workspaces.is_empty()).then_some(MouseAction::AttachNext),
        ),
        (
            &state.config.keymap.workspace_overlay,
            "open the workspace picker",
            (!state.snapshot.workspaces.is_empty()).then_some(MouseAction::OpenPicker),
        ),
    ] {
        let line_index = lines.len();
        let chord = chord_label(chord);
        let padding = syntax_column.saturating_sub(chord.chars().count());
        lines.push(Line::from(vec![
            Span::raw(chord),
            Span::raw(" ".repeat(padding + COLUMN_GAP)),
            Span::styled(meaning, section_style(state)),
        ]));
        if let Some(action) = action {
            row_actions.push((line_index, action));
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .style(ui_style(state))
            .scroll((scroll, 0)),
        rows[1],
    );
    state.mouse_targets.push(MouseTarget {
        area: rows[1],
        action: MouseAction::ScrollHelp,
    });
    let close_x = 5 + hint_pad;
    if close_x < usize::from(rows[0].width) {
        state.mouse_targets.push(MouseTarget {
            area: Rect::new(
                rows[0].x.saturating_add(close_x as u16),
                rows[0].y,
                Line::raw("esc close")
                    .width()
                    .min(usize::from(rows[0].width) - close_x) as u16,
                1,
            ),
            action: MouseAction::CloseHelp,
        });
    }
    state
        .mouse_targets
        .extend(row_actions.into_iter().filter_map(|(line, action)| {
            let visible = line.checked_sub(usize::from(scroll))?;
            (visible < usize::from(rows[1].height)).then_some(MouseTarget {
                area: Rect::new(
                    rows[1].x,
                    rows[1].y.saturating_add(visible as u16),
                    rows[1].width,
                    1,
                ),
                action,
            })
        }));
}

/// Row count of the rendered help, used to clamp scrolling.
pub(super) fn help_rows(state: &ClientState) -> usize {
    let catalog = entries(state);
    let groups = CommandGroup::ORDER
        .iter()
        .filter(|group| catalog.iter().any(|entry| entry.group == **group))
        .count();
    // Each group contributes a heading plus a blank spacer, and the trailing
    // KEYS block adds its own heading, spacer, and three chord rows.
    catalog.len() + groups * 2 + 6
}
