use super::glyph::Glyphs;
use super::style::{danger_style, panel_style, section_style, warning_style};
use crate::client::ClientState;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub(super) fn render_ports(state: &mut ClientState, frame: &mut ratatui::Frame, area: Rect) {
    state.ports_area = Some(area);
    state.port_click_targets.clear();
    let (lines, targets) = port_lines(state, area.width);
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
    // The count belongs on the right of the section label, matching the
    // session and hosts columns.
    let heading = if max_scroll == 0 {
        Line::styled("PORTS", section_style(state))
    } else {
        let count = format!("{}/{}", scroll + 1, max_scroll + 1);
        let padding = usize::from(area.width).saturating_sub(5 + count.chars().count());
        Line::styled(
            format!("PORTS{}{count}", " ".repeat(padding)),
            section_style(state),
        )
    };
    frame.render_widget(Paragraph::new(heading).style(panel_style(state)), rows[0]);
    frame.render_widget(
        Paragraph::new(lines)
            .style(panel_style(state))
            .scroll((state.ports_scroll, 0)),
        rows[1],
    );
}

type ClickTarget = Option<(crate::core::WorkspaceId, crate::ports::RemotePortTarget)>;

fn port_lines(state: &ClientState, width: u16) -> (Vec<Line<'static>>, Vec<ClickTarget>) {
    let glyphs = Glyphs::of(state);
    let mut lines = Vec::new();
    let mut targets = Vec::new();
    let active_workspace = state.selected_workspace.or(state.active_workspace);
    let active_host = active_workspace.and_then(|id| state.host_for_workspace(id));
    if let Some(snapshot) = active_host.and_then(|host_id| state.ports.get(&host_id)) {
        if let Some(warning) = &snapshot.warning {
            lines.push(Line::styled(
                format!("{} {warning}", glyphs.warning()),
                warning_style(state),
            ));
            lines.push(Line::styled(":ports --all-host", section_style(state)));
            targets.extend([None, None]);
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
            let (action, action_style, clickable) = if let Some(forward) = forward {
                match &forward.status {
                    crate::ports::ForwardStatus::Direct => (
                        format!("{} {}", glyphs.arrow(), forward.local_address.port()),
                        Style::default(),
                        None,
                    ),
                    crate::ports::ForwardStatus::Active => (
                        format!("{} {}", glyphs.arrow(), forward.local_address.port()),
                        Style::default(),
                        None,
                    ),
                    crate::ports::ForwardStatus::Reconnecting => {
                        ("reconnecting".to_owned(), warning_style(state), None)
                    }
                    crate::ports::ForwardStatus::Cancelling => {
                        ("cancelling".to_owned(), section_style(state), None)
                    }
                    crate::ports::ForwardStatus::PortConflict => {
                        ("conflict".to_owned(), danger_style(state), None)
                    }
                    crate::ports::ForwardStatus::Failed(reason) => (
                        format!("failed: {}", reason.chars().take(24).collect::<String>()),
                        danger_style(state),
                        None,
                    ),
                }
            } else if ambiguous {
                ("ambiguous".to_owned(), danger_style(state), None)
            } else if target.is_none() {
                ("invalid address".to_owned(), danger_style(state), None)
            } else {
                (
                    "click to forward".to_owned(),
                    section_style(state),
                    active_workspace.zip(target),
                )
            };

            let port = listener.port.to_string();
            let padding = usize::from(width)
                .saturating_sub(port.chars().count() + Line::raw(&action).width())
                .max(1);
            lines.push(Line::from(vec![
                Span::raw(port),
                Span::raw(" ".repeat(padding)),
                Span::styled(action, action_style),
            ]));
            targets.push(clickable);
            // The detail row is never clickable; the port row above owns the
            // whole listener's hit target.
            lines.push(Line::styled(
                fit(
                    glyphs,
                    &listener_detail(glyphs, listener, process),
                    usize::from(width),
                ),
                section_style(state),
            ));
            targets.push(None);
        }
    }
    if lines.is_empty() {
        lines.extend([
            Line::raw("No workspace ports found."),
            Line::styled(":ports --all-host", section_style(state)),
        ]);
        targets.extend([None, None]);
    }
    debug_assert_eq!(lines.len(), targets.len());
    (lines, targets)
}

/// The dim second row: who is listening and on which address. Two rows per
/// listener keeps the port and its forward state on one scannable column.
fn listener_detail(glyphs: Glyphs, listener: &crate::ports::PortListener, process: &str) -> String {
    format!(
        "{process} {} {}:{}",
        glyphs.separator(),
        listener.bind_address,
        listener.port
    )
}

fn fit(glyphs: Glyphs, value: &str, columns: usize) -> String {
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
