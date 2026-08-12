use super::style::ui_style;
use crate::client::{ClientMode, ClientState};
use crate::ports::{ForwardStatus, ProbeCompleteness};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub(super) fn render_footer(state: &ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let mode = match state.mode {
        ClientMode::Work => Span::styled(" WORK ", Style::default().fg(Color::DarkGray)),
        ClientMode::Manage => Span::styled(
            " MANAGE ",
            Style::default().bg(Color::Magenta).fg(Color::Black),
        ),
        ClientMode::Authenticate => Span::styled(
            " AUTHENTICATE ",
            Style::default().bg(Color::Yellow).fg(Color::Black),
        ),
    };
    let lines = if state.mode == ClientMode::Work {
        vec![work_footer(state, mode)]
    } else if state.command_active {
        vec![
            Line::from(vec![
                Span::styled(&state.command_input, Style::default().fg(Color::White)),
                Span::styled(" ", Style::default().bg(Color::White)),
            ]),
            Line::raw(" Enter run  Esc cancel"),
        ]
    } else if let Some(output) = state.visible_output() {
        vec![
            Line::from(vec![mode, Span::raw(" "), Span::raw(output)]),
            Line::raw(default_footer_hint(state)),
        ]
    } else {
        vec![Line::from(vec![
            mode,
            Span::raw(default_footer_hint(state)),
        ])]
    };
    frame.render_widget(Paragraph::new(lines).style(ui_style(state)), area);
}

fn work_footer(state: &ClientState, mode: Span<'static>) -> Line<'static> {
    let manage = format!("  {} Manage", state.config.keymap.toggle_mode);
    let mut spans = vec![mode, Span::raw(manage)];
    if let Some(output) = state.visible_output() {
        spans.push(Span::raw("  ·  "));
        spans.push(Span::raw(output.to_owned()));
        return Line::from(spans);
    }
    let attention = work_attention(state);
    if !attention.is_empty() {
        spans.push(Span::styled(
            format!("  ·  ⚠ {}", attention.join(" · ")),
            Style::default().fg(Color::Yellow),
        ));
    }
    spans.push(Span::raw(format!(
        "  ·  {} Next  ·  {} List",
        state.config.keymap.switch_workspace, state.config.keymap.workspace_overlay,
    )));
    Line::from(spans)
}

pub(super) fn work_attention(state: &ClientState) -> Vec<String> {
    let Some(workspace_id) = state.active_workspace else {
        return Vec::new();
    };
    let mut attention = Vec::new();
    match state.statuses.get(&workspace_id).copied() {
        Some(crate::client::DisplayStatus::NeedsInput) => attention.push("input needed".to_owned()),
        Some(crate::client::DisplayStatus::Done) => attention.push("agent done".to_owned()),
        Some(crate::client::DisplayStatus::Unknown) => {
            attention.push("agent status unknown".to_owned())
        }
        Some(crate::client::DisplayStatus::Exited) => attention.push("agent exited".to_owned()),
        _ => {}
    }
    if let Some(clients) = state
        .connected_clients
        .get(&workspace_id)
        .copied()
        .filter(|clients| *clients > 1)
    {
        attention.push(format!("{clients} clients"));
    }
    let port_probe_needs_attention = state
        .host_for_workspace(workspace_id)
        .and_then(|host_id| state.ports.get(&host_id))
        .is_some_and(|snapshot| {
            snapshot.warning.is_some() || snapshot.completeness != ProbeCompleteness::Full
        });
    let forward_needs_attention = state.forwards.iter().any(|forward| {
        forward.workspace_id == workspace_id
            && !matches!(
                forward.status,
                ForwardStatus::Active | ForwardStatus::Direct
            )
    });
    if port_probe_needs_attention || forward_needs_attention {
        attention.push("ports".to_owned());
    }
    attention
}

pub(super) fn default_footer_hint(state: &ClientState) -> String {
    match state.mode {
        ClientMode::Work => format!(
            "  {} Manage  {} Next  {} List",
            state.config.keymap.toggle_mode,
            state.config.keymap.switch_workspace,
            state.config.keymap.workspace_overlay,
        ),
        ClientMode::Manage if !state.host_operations.is_empty() => {
            "  Esc cancel selected-host operation  : commands  q quit".to_owned()
        }
        ClientMode::Manage => "  : commands  ↑↓ select  Enter attach  q quit".to_owned(),
        ClientMode::Authenticate => "  Respond to the SSH prompt; Ctrl+C cancels".to_owned(),
    }
}
