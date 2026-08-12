use super::style::{
    connection_style, connection_symbol, danger_style, panel_style, section_style, selected_style,
    status_span, warning_style,
};
use crate::client::{ClientState, DisplayStatus};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub(super) fn render_sidebar(state: &ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let mut lines = vec![Line::styled("HOSTS", section_style(state))];
    let mut selected_last_line = None;
    for host in &state.tree {
        let operation = state
            .host_operations
            .get(&host.id)
            .map(|(_, label)| format!("  ◐ {label}"));
        let symbol = connection_symbol(host.connection);
        let (label, padding) = aligned_label(&host.label, host.status, 2, area.width);
        lines.push(Line::from(vec![
            Span::styled(symbol, connection_style(state, host.connection)),
            Span::raw(" "),
            Span::styled(label, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(padding),
            status_span(state, host.status),
        ]));
        if let Some(operation) = operation {
            lines.push(Line::styled(
                fit_to_columns(&operation, usize::from(area.width)),
                warning_style(state),
            ));
        }
        for repository in &host.repositories {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("▾ ", section_style(state)),
                Span::raw(&repository.label),
            ]));
            for workspace in &repository.workspaces {
                let selected = state.selected_workspace == Some(workspace.id);
                let active = state.active_workspace == Some(workspace.id);
                let marker = if active { "●" } else { " " };
                let (label, padding) =
                    aligned_label(&workspace.label, workspace.status, 6, area.width);
                if selected {
                    lines.push(Line::styled(
                        format!(
                            "    {marker} {label}{padding}{}",
                            workspace.status.public_text(),
                        ),
                        selected_style(state),
                    ));
                } else {
                    let workspace_style = if workspace.setup_failed {
                        danger_style(state)
                    } else {
                        Style::default()
                    };
                    lines.push(Line::from(vec![
                        Span::raw(format!("    {marker} ")),
                        Span::styled(label, workspace_style),
                        Span::raw(padding),
                        status_span(state, workspace.status),
                    ]));
                }
                // Setup failure is durable workspace state, not an agent
                // status. Give it a full semantic row so neither meaning is
                // clipped inside the fixed 32-column navigation surface.
                if workspace.setup_failed {
                    lines.push(Line::styled("      ⚠ setup failed", danger_style(state)));
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
            Line::raw("No workspaces registered."),
            Line::raw(":workspace add <path>"),
            Line::raw(":host add <name> <alias>"),
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

fn aligned_label(
    label: &str,
    status: DisplayStatus,
    prefix_width: usize,
    row_width: u16,
) -> (String, String) {
    let status_width = Line::raw(status.public_text()).width();
    let row_width = usize::from(row_width);
    let label_width = row_width.saturating_sub(prefix_width + status_width + 2);
    let label = fit_to_columns(label, label_width);
    let padding = row_width.saturating_sub(prefix_width + Line::raw(&label).width() + status_width);
    (label, " ".repeat(padding))
}

fn fit_to_columns(value: &str, columns: usize) -> String {
    if Line::raw(value).width() <= columns {
        return value.to_owned();
    }
    if columns == 0 {
        return String::new();
    }
    let mut output = String::new();
    for character in value.chars() {
        let candidate = format!("{output}{character}…");
        if Line::raw(&candidate).width() > columns {
            break;
        }
        output.push(character);
    }
    output.push('…');
    output
}
