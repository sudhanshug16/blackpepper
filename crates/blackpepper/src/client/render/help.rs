//! Grouped `:help`.
//!
//! Commands are grouped by what they act on, current context first, and every
//! entry carries a right-hand column describing the state it would act against.
//! A command that cannot run right now stays listed and goes dim with the
//! reason in that column — the list never lies about what will run.

use super::glyph::Glyphs;
use super::style::{accent_style, section_style, ui_style};
use crate::client::catalog::{entries, CommandGroup};
use crate::client::ClientState;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Width of the syntax column. Wide enough for the longest entry so the notes
/// form a single readable right column.
const SYNTAX_COLUMN: usize = 34;

pub(super) fn render_help(state: &ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let glyphs = Glyphs::of(state);
    // Clamp rather than let the view scroll past its own end, which would
    // otherwise leave an empty pane and no obvious way back.
    let visible = usize::from(area.height).saturating_sub(1);
    let max_scroll = help_rows(state).saturating_sub(visible);
    let scroll = state
        .help
        .map(|help| usize::from(help.scroll).min(max_scroll) as u16)
        .unwrap_or_default();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let separator = glyphs.separator();
    let heading = Line::from(vec![
        Span::styled(":", accent_style(state)),
        Span::raw("help"),
        Span::styled(
            format!("  esc close {separator} {} scroll", glyphs.updown()),
            section_style(state),
        ),
    ]);
    frame.render_widget(Paragraph::new(heading).style(ui_style(state)), rows[0]);

    let catalog = entries(state);
    let mut lines = Vec::new();
    for group in CommandGroup::ORDER {
        let group_entries = catalog
            .iter()
            .filter(|entry| entry.group == group)
            .collect::<Vec<_>>();
        if group_entries.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(group.heading(), section_style(state)));
        for entry in group_entries {
            let syntax = format!("{:SYNTAX_COLUMN$}", entry.syntax);
            let syntax_style = if entry.available {
                Style::default()
            } else {
                section_style(state)
            };
            lines.push(Line::from(vec![
                Span::styled(syntax, syntax_style),
                Span::styled(entry.note.clone(), section_style(state)),
            ]));
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("KEYS", section_style(state)));
    for (chord, meaning) in [
        (&state.config.keymap.toggle_mode, "switch work / manage"),
        (
            &state.config.keymap.switch_workspace,
            "attach next workspace",
        ),
        (
            &state.config.keymap.workspace_overlay,
            "open the workspace picker",
        ),
    ] {
        lines.push(Line::from(vec![
            Span::raw(format!("{chord:SYNTAX_COLUMN$}")),
            Span::styled(meaning, section_style(state)),
        ]));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .style(ui_style(state))
            .scroll((scroll, 0)),
        rows[1],
    );
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
    catalog.len() + groups * 2 + 5
}
