use super::style::{panel_block, panel_style};
use crate::client::ClientState;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

pub(super) fn render_ports(state: &mut ClientState, frame: &mut ratatui::Frame, area: Rect) {
    state.ports_area = Some(area);
    state.port_click_targets.clear();
    let mut lines = Vec::new();
    let mut targets = Vec::new();
    let active_workspace = state.selected_workspace.or(state.active_workspace);
    let active_host = active_workspace.and_then(|id| state.host_for_workspace(id));
    if let Some(host_id) = active_host {
        if let Some(snapshot) = state.ports.get(&host_id) {
            if let Some(warning) = &snapshot.warning {
                lines.push(Line::styled(
                    format!("⚠ {warning}"),
                    Style::default().fg(Color::Yellow),
                ));
                targets.push(None);
                lines.push(Line::raw(""));
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
                if state.show_all_host_ports || in_workspace {
                    let target = listener.forward_target().ok();
                    let target_is_ambiguous = target.as_ref().is_some_and(|target| {
                        crate::ports::target_is_ambiguous(&snapshot.listeners, target)
                    });
                    let forward = target.as_ref().and_then(|target| {
                        state.forwards.iter().find(|forward| {
                            active_workspace == Some(forward.workspace_id)
                                && forward.target() == *target
                        })
                    });
                    let map = forward
                        .map(|forward| {
                            let status = match &forward.status {
                                crate::ports::ForwardStatus::Direct => " direct".to_string(),
                                crate::ports::ForwardStatus::Active => String::new(),
                                crate::ports::ForwardStatus::Reconnecting => {
                                    " reconnecting".to_string()
                                }
                                crate::ports::ForwardStatus::Cancelling => {
                                    " cancelling".to_string()
                                }
                                crate::ports::ForwardStatus::PortConflict => {
                                    " PORT CONFLICT".to_string()
                                }
                                crate::ports::ForwardStatus::Failed(reason) => format!(
                                    " FAILED: {}",
                                    reason.chars().take(80).collect::<String>()
                                ),
                            };
                            format!(" → {}{status}", forward.local_address.port())
                        })
                        .unwrap_or_default();
                    let ambiguity = if target_is_ambiguous {
                        " AMBIGUOUS (shared socket)"
                    } else if target.is_none() {
                        " INVALID ADDRESS"
                    } else {
                        ""
                    };
                    lines.push(Line::raw(format!(
                        "{}{} {}{}",
                        listener.bind_endpoint(),
                        map,
                        listener.process.as_deref().unwrap_or("unknown"),
                        ambiguity,
                    )));
                    targets.push(active_workspace.zip(target.filter(|_| !target_is_ambiguous)));
                }
            }
        }
    }
    if lines.is_empty() {
        lines.push(Line::raw("No workspace ports found."));
        targets.push(None);
        lines.push(Line::raw(""));
        targets.push(None);
        lines.push(Line::raw(":ports --all-host"));
        targets.push(None);
    }

    let visible_rows = area.height.saturating_sub(2) as usize;
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
        let row = area
            .y
            .saturating_add(1)
            .saturating_add((index - scroll) as u16);
        state
            .port_click_targets
            .push(crate::client::state::PortClickTarget {
                workspace_id,
                target,
                x_start: area.x.saturating_add(1),
                x_end: area.x.saturating_add(area.width.saturating_sub(1)),
                y: row,
            });
    }
    let title = if max_scroll == 0 {
        " Ports ".to_owned()
    } else {
        format!(
            " Ports · wheel {}/{} ",
            scroll.saturating_add(1),
            max_scroll + 1
        )
    };
    frame.render_widget(
        Paragraph::new(lines)
            .style(panel_style(state))
            .block(panel_block(state).title(title))
            .scroll((state.ports_scroll, 0)),
        area,
    );
}
