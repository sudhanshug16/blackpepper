//! The workspace picker overlay.
//!
//! This is the one surface that flattens every host into a single list, so
//! switching does not require knowing which machine a workspace lives on. It
//! draws over the session column and leaves the sidebar and status row in
//! place, keeping the mode and workspace anchors visible while it is open.

use super::glyph::Glyphs;
use super::style::{panel_style, section_style, selected_style, status_span, status_text};
use crate::client::ClientState;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

pub(super) fn render_picker(state: &ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let glyphs = Glyphs::of(state);
    let separator = glyphs.separator();
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

    let mut lines = vec![Line::from(vec![
        Span::styled("SWITCH TO", section_style(state)),
        Span::raw("  "),
        Span::raw(filter),
        Span::styled(
            " ",
            ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
        ),
    ])];

    if matches.is_empty() {
        lines.push(Line::styled(
            "no workspace matches that filter",
            section_style(state),
        ));
    }

    // Reserve the heading and the trailing hint row.
    let visible = usize::from(area.height).saturating_sub(3);
    let offset = selected.saturating_sub(visible.saturating_sub(1));
    for (index, (workspace_id, label, host, status)) in
        matches.iter().enumerate().skip(offset).take(visible)
    {
        let detail = state.status_detail(*workspace_id, *status);
        let status_text = status_text(state, *status, detail.as_deref());
        let right = format!("{host} {separator} {status_text}");
        let width = usize::from(area.width);
        let padding = width
            .saturating_sub(2 + label.chars().count() + Line::raw(&right).width())
            .max(1);
        if index == selected {
            lines.push(Line::styled(
                format!("  {label}{}{right}", " ".repeat(padding)),
                selected_style(state),
            ));
        } else {
            lines.push(Line::from(vec![
                Span::raw(format!("  {label}{}", " ".repeat(padding))),
                Span::styled(format!("{host} {separator} "), section_style(state)),
                status_span(state, *status, detail.as_deref()),
            ]));
        }
    }

    lines.push(Line::styled(
        format!("  type to filter {separator} enter attach {separator} esc cancel"),
        section_style(state),
    ));

    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).style(panel_style(state)), area);
}
