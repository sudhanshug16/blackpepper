use super::style::{
    connected_client_label, connection_style, connection_symbol, panel_block, panel_style,
    selected_style, status_span,
};
use crate::client::ClientState;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub(super) fn render_sidebar(state: &ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let mut lines = Vec::new();
    let mut selected_line = None;
    for host in &state.tree {
        let operation = state
            .host_operations
            .get(&host.id)
            .map(|(_, label)| format!("  ◐ {label}"))
            .unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled(
                connection_symbol(host.connection),
                connection_style(host.connection),
            ),
            Span::raw(" "),
            Span::styled(&host.label, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            status_span(host.status),
            Span::styled(operation, Style::default().fg(Color::Yellow)),
        ]));
        for repository in &host.repositories {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("▾ ", Style::default().fg(Color::DarkGray)),
                Span::raw(&repository.label),
            ]));
            for workspace in &repository.workspaces {
                let selected = state.selected_workspace == Some(workspace.id);
                let active = state.active_workspace == Some(workspace.id);
                let style = if selected {
                    selected_style(state)
                } else {
                    Style::default()
                };
                let marker = if active { "●" } else { " " };
                let setup = if workspace.setup_failed {
                    " setup-failed"
                } else {
                    ""
                };
                let clients =
                    connected_client_label(state.connected_clients.get(&workspace.id).copied());
                if selected {
                    selected_line = Some(lines.len());
                }
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("    {marker} {}{setup}{clients} ", workspace.label),
                        if workspace.setup_failed {
                            style.fg(Color::Red)
                        } else {
                            style
                        },
                    ),
                    status_span(workspace.status),
                ]));
            }
        }
    }
    if lines.is_empty() {
        lines.extend([
            Line::raw("No workspaces registered."),
            Line::raw(""),
            Line::raw(":workspace add <path>"),
            Line::raw(":host add <name> <alias>"),
        ]);
    }
    let visible_rows = area.height.saturating_sub(2) as usize;
    let scroll = selected_line
        .map(|line| line.saturating_sub(visible_rows.saturating_sub(1)))
        .unwrap_or_default()
        .min(u16::MAX as usize) as u16;
    frame.render_widget(
        Paragraph::new(lines)
            .style(panel_style(state))
            .scroll((scroll, 0))
            .block(panel_block(state).title(" Hosts / Workspaces ")),
        area,
    );
}
