//! The workspace picker overlay.
//!
//! This is the one surface that flattens every host into a single list, so
//! switching does not require knowing which machine a workspace lives on. It
//! draws over the session column and leaves the sidebar and status row in
//! place, keeping the mode and workspace anchors visible while it is open.

use super::chrome;
use super::glyph::Glyphs;
use super::style::{
    list_status_text, mid_style, panel_style, section_style, selected_style, status_style,
};
use crate::client::ClientState;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

pub(super) fn render_picker(state: &ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let glyphs = Glyphs::of(state);
    let separator = glyphs.separator();
    let pad = chrome::pad(area.width);
    let inner = chrome::inner_width(area.width);
    let matches = state.picker_matches();
    let selected = state
        .picker
        .as_ref()
        .map(|picker| picker.selected)
        .unwrap_or_default();
    let filter = state
        .picker
        .as_ref()
        .map(|picker| picker.filter.clone())
        .unwrap_or_default();

    // Heading and filter echo are separate rows: at 30 columns a filter typed
    // beside the label would collide with it.
    let mut lines = vec![
        Line::styled(format!("{pad}SWITCH TO"), section_style(state)),
        Line::from(vec![
            Span::raw(format!("{pad}{filter}")),
            Span::styled(" ", Style::default().add_modifier(Modifier::REVERSED)),
        ]),
    ];

    if matches.is_empty() {
        lines.push(Line::styled(
            format!("{pad}no workspace matches that filter"),
            section_style(state),
        ));
    }

    // Reserve the two chrome rows above and the hint row below.
    let visible = usize::from(area.height).saturating_sub(3);
    let offset = selected.saturating_sub(visible.saturating_sub(1));
    for (index, (workspace_id, label, host, status)) in
        matches.iter().enumerate().skip(offset).take(visible)
    {
        let detail = state.status_elapsed(*workspace_id, *status);
        let right = format!(
            "{host} {separator} {}",
            list_status_text(state, *status, detail.as_deref())
        );
        let padding = inner
            .saturating_sub(Line::raw(label).width() + Line::raw(&right).width())
            .max(1);
        if index == selected {
            // Full-bleed: the highlight runs through the gutter to both edges.
            lines.push(Line::styled(
                format!("{pad}{label}{}{right}{pad}", " ".repeat(padding)),
                selected_style(state),
            ));
        } else {
            lines.push(Line::from(vec![
                Span::raw(pad.clone()),
                Span::styled(label.clone(), mid_style(state)),
                Span::raw(" ".repeat(padding)),
                // Host and status read as one cluster in the status colour, so
                // a row that needs attention is legible without a second look.
                Span::styled(right, status_style(state, *status)),
                Span::raw(pad.clone()),
            ]));
        }
    }

    lines.push(Line::styled(
        format!("{pad}type to filter {separator} enter attach {separator} esc cancel"),
        section_style(state),
    ));

    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).style(panel_style(state)), area);
}
