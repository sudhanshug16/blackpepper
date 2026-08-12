use super::style::{panel_style, section_style, warning_style};
use crate::client::ClientState;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

pub(super) fn render_ports(state: &mut ClientState, frame: &mut ratatui::Frame, area: Rect) {
    state.ports_area = Some(area);
    state.port_click_targets.clear();
    let (lines, targets) = port_lines(state);
    let visible_rows = area.height.saturating_sub(1) as usize;
    let max_scroll = lines.len().saturating_sub(visible_rows);
    state.ports_scroll = usize::from(state.ports_scroll)
        .min(max_scroll)
        .min(usize::from(u16::MAX)) as u16;
    let scroll = usize::from(state.ports_scroll);

    for (index, target) in targets.into_iter().enumerate() {
        let Some((workspace_id, target)) = target else {
            continue;
        };
        if index < scroll || index >= scroll.saturating_add(visible_rows) {
            continue;
        }
        state
            .port_click_targets
            .push(crate::client::state::PortClickTarget {
                workspace_id,
                target,
                x_start: area.x,
                x_end: area.x.saturating_add(area.width),
                y: area
                    .y
                    .saturating_add(1)
                    .saturating_add((index - scroll) as u16),
            });
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);
    let heading = if max_scroll == 0 {
        "PORTS".to_owned()
    } else {
        format!("PORTS  {}/{}", scroll + 1, max_scroll + 1)
    };
    frame.render_widget(
        Paragraph::new(Line::styled(heading, section_style(state))).style(panel_style(state)),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(lines)
            .style(panel_style(state))
            .scroll((state.ports_scroll, 0)),
        rows[1],
    );
}

type ClickTarget = Option<(crate::core::WorkspaceId, crate::ports::RemotePortTarget)>;

fn port_lines(state: &ClientState) -> (Vec<Line<'static>>, Vec<ClickTarget>) {
    let mut lines = Vec::new();
    let mut targets = Vec::new();
    let active_workspace = state.selected_workspace.or(state.active_workspace);
    let active_host = active_workspace.and_then(|id| state.host_for_workspace(id));
    if let Some(snapshot) = active_host.and_then(|host_id| state.ports.get(&host_id)) {
        if let Some(warning) = &snapshot.warning {
            lines.push(Line::styled(format!("⚠ {warning}"), warning_style(state)));
            targets.push(None);
        }
        let workspace_root = active_workspace.and_then(|workspace_id| {
            state
                .snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.id == workspace_id)
                .map(|workspace| workspace.root_path.as_str())
        });
        for listener in &snapshot.listeners {
            let in_workspace = workspace_root.is_some_and(|root| {
                crate::client::runtime::ports::listener_matches_workspace(
                    listener.workspace_path.as_deref(),
                    root,
                )
            });
            if !state.show_all_host_ports && !in_workspace {
                continue;
            }
            let target = listener.forward_target().ok();
            let ambiguous = target.as_ref().is_some_and(|target| {
                crate::ports::target_is_ambiguous(&snapshot.listeners, target)
            });
            let forward = target.as_ref().and_then(|target| {
                state.forwards.iter().find(|forward| {
                    active_workspace == Some(forward.workspace_id) && forward.target() == *target
                })
            });
            let process = listener.process.as_deref().unwrap_or("unknown");
            let (line, clickable) = if let Some(forward) = forward {
                let status = match &forward.status {
                    crate::ports::ForwardStatus::Direct => "direct".to_owned(),
                    crate::ports::ForwardStatus::Active => "active".to_owned(),
                    crate::ports::ForwardStatus::Reconnecting => "reconnecting".to_owned(),
                    crate::ports::ForwardStatus::Cancelling => "cancelling".to_owned(),
                    crate::ports::ForwardStatus::PortConflict => "conflict".to_owned(),
                    crate::ports::ForwardStatus::Failed(reason) => {
                        format!("failed: {}", reason.chars().take(40).collect::<String>())
                    }
                };
                (
                    format!(
                        "{} → {} {status} · {process}",
                        listener.port,
                        forward.local_address.port()
                    ),
                    None,
                )
            } else if ambiguous {
                (
                    format!("{} ambiguous shared socket · {process}", listener.port),
                    None,
                )
            } else if target.is_none() {
                (
                    format!("{} invalid address · {process}", listener.port),
                    None,
                )
            } else {
                (
                    format!("{} click to forward · {process}", listener.port),
                    active_workspace.zip(target),
                )
            };
            lines.push(Line::raw(line));
            targets.push(clickable);
        }
    }
    if lines.is_empty() {
        lines.extend([
            Line::raw("No workspace ports found."),
            Line::raw(":ports --all-host"),
        ]);
        targets.extend([None, None]);
    }
    (lines, targets)
}
