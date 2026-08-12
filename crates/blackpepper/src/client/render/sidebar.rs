use super::chrome;
use super::glyph::Glyphs;
use super::style::{
    accent_style, connection_style, danger_style, list_status_span, list_status_text, mid_style,
    panel_style, section_style, selected_style,
};
use crate::client::{ClientState, HostConnection};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub(super) fn render_sidebar(state: &ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let glyphs = Glyphs::of(state);
    // The panel keeps its full rect so `panel_style` paints the gutter cells;
    // each line carries its own inset instead.
    let pad = chrome::pad(area.width);
    let inner = chrome::inner_width(area.width);
    let mut lines = vec![Line::styled(format!("{pad}HOSTS"), section_style(state))];
    let mut selected_last_line = None;
    for (index, host) in state.tree.iter().enumerate() {
        // One blank row between hosts, so a long tree reads as groups rather
        // than as one undifferentiated column.
        if index > 0 {
            lines.push(Line::raw(""));
        }
        // The host row reports reachability. Agent state belongs to the
        // workspace rows below it, so the two columns never compete for the
        // same meaning. Only the glyph carries colour; the word stays dim.
        let connection = host.connection.public_word();
        let (label, padding) = aligned_label(glyphs, &host.label, connection, 2, inner);
        let label_style = if host.connection == HostConnection::Disconnected {
            section_style(state)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::raw(pad.clone()),
            Span::styled(
                glyphs.connection(host.connection),
                connection_style(state, host.connection),
            ),
            Span::raw(" "),
            Span::styled(label, label_style),
            Span::raw(padding),
            Span::styled(connection, section_style(state)),
            Span::raw(pad.clone()),
        ]));
        if let Some((_, operation)) = state.host_operations.get(&host.id) {
            lines.push(Line::from(vec![
                Span::raw(format!("{pad}  ")),
                Span::styled(glyphs.spinner(state.spinner_phase()), accent_style(state)),
                Span::raw(" "),
                Span::styled(
                    fit_to_columns(glyphs, operation, inner.saturating_sub(4)),
                    section_style(state),
                ),
            ]));
        }
        for repository in &host.repositories {
            // A dim label and one indent step separate the group; the design
            // spends no glyph on a disclosure marker that never toggles.
            lines.push(Line::from(vec![
                Span::raw(format!("{pad}  ")),
                Span::styled(
                    fit_to_columns(glyphs, &repository.label, inner.saturating_sub(2)),
                    section_style(state),
                ),
            ]));
            for workspace in &repository.workspaces {
                let selected = state.selected_workspace == Some(workspace.id);
                let active = state.active_workspace == Some(workspace.id);
                let marker = if active { glyphs.connected() } else { " " };
                let detail = state.status_elapsed(workspace.id, workspace.status);
                let status = list_status_text(state, workspace.status, detail.as_deref());
                let (label, padding) = aligned_label(glyphs, &workspace.label, &status, 3, inner);
                if selected {
                    // One span across the whole row: the design bleeds the
                    // selection through the gutter to both panel edges.
                    lines.push(Line::styled(
                        format!("{pad}{marker}  {label}{padding}{status}{pad}"),
                        selected_style(state),
                    ));
                } else {
                    let workspace_style = if workspace.setup_failed {
                        danger_style(state)
                    } else {
                        mid_style(state)
                    };
                    lines.push(Line::from(vec![
                        Span::raw(format!("{pad}{marker}  ")),
                        Span::styled(label, workspace_style),
                        Span::raw(padding),
                        list_status_span(state, workspace.status, detail.as_deref()),
                        Span::raw(pad.clone()),
                    ]));
                }
                // Setup failure is durable workspace state, not an agent
                // status. Give it a full semantic row so neither meaning is
                // clipped inside the fixed 32-column navigation surface.
                if workspace.setup_failed {
                    lines.push(Line::styled(
                        format!("{pad}    {} setup failed", glyphs.warning()),
                        danger_style(state),
                    ));
                }
                if selected {
                    // Scroll through the durable setup warning when present,
                    // keeping the selected row and its following semantic row
                    // together in compact selectors.
                    selected_last_line = Some(lines.len().saturating_sub(1));
                }
            }
        }
    }
    if state.tree.is_empty() {
        lines.extend([
            Line::raw(format!("{pad}No workspaces registered.")),
            Line::raw(format!("{pad}:workspace add <path>")),
            Line::raw(format!("{pad}:host add <name> <alias>")),
        ]);
    }
    let visible_rows = area.height as usize;
    let scroll = selected_last_line
        .map(|line| line.saturating_sub(visible_rows.saturating_sub(1)))
        .unwrap_or_default()
        .min(u16::MAX as usize) as u16;
    frame.render_widget(
        Paragraph::new(lines)
            .style(panel_style(state))
            .scroll((scroll, 0)),
        area,
    );
}

/// Fit `label` and right-align `trailing` within the panel's inner width.
/// `prefix_width` counts the marker columns already spent before the label.
fn aligned_label(
    glyphs: Glyphs,
    label: &str,
    trailing: &str,
    prefix_width: usize,
    inner_width: usize,
) -> (String, String) {
    let trailing_width = Line::raw(trailing).width();
    let label_width = inner_width.saturating_sub(prefix_width + trailing_width + 2);
    let label = fit_to_columns(glyphs, label, label_width);
    let padding =
        inner_width.saturating_sub(prefix_width + Line::raw(&label).width() + trailing_width);
    (label, " ".repeat(padding))
}

fn fit_to_columns(glyphs: Glyphs, value: &str, columns: usize) -> String {
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
