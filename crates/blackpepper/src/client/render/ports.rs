use super::chrome;
use super::glyph::Glyphs;
use super::style::{danger_style, panel_style, section_style, warning_style};
use crate::client::state::{MouseAction, MouseTarget};
use crate::client::ClientState;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub(super) fn render_ports(state: &mut ClientState, frame: &mut ratatui::Frame, area: Rect) {
    state.ports_area = Some(area);
    let (lines, targets) = port_lines(state, area.width);
    let visible_rows = area.height.saturating_sub(1) as usize;
    let max_scroll = lines.len().saturating_sub(visible_rows);
    state.ports_scroll = usize::from(state.ports_scroll)
        .min(max_scroll)
        .min(usize::from(u16::MAX)) as u16;
    let scroll = usize::from(state.ports_scroll);

    state.mouse_targets.push(MouseTarget {
        area,
        action: MouseAction::ScrollPorts,
    });
    for (index, action) in targets.into_iter().enumerate() {
        let Some(action) = action else {
            continue;
        };
        if index < scroll || index >= scroll.saturating_add(visible_rows) {
            continue;
        }
        state.mouse_targets.push(MouseTarget {
            area: Rect::new(
                area.x,
                area.y
                    .saturating_add(1)
                    .saturating_add((index - scroll) as u16),
                area.width,
                1,
            ),
            action,
        });
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);
    // The count belongs on the right of the section label, matching the
    // session and hosts columns.
    let pad = chrome::pad(area.width);
    let inner = chrome::inner_width(area.width);
    let heading = if max_scroll == 0 {
        Line::styled(format!("{pad}PORTS"), section_style(state))
    } else {
        let count = format!("{}/{}", scroll + 1, max_scroll + 1);
        Line::styled(
            format!(
                "{pad}{}{pad}",
                chrome::right_aligned("PORTS", &count, inner)
            ),
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

type RowAction = Option<MouseAction>;

fn port_lines(state: &ClientState, width: u16) -> (Vec<Line<'static>>, Vec<RowAction>) {
    let glyphs = Glyphs::of(state);
    let mut lines = Vec::new();
    let mut targets = Vec::new();
    let mut listener_rows = 0usize;
    let mut all_host_hint_shown = false;
    let active_workspace = state.selected_workspace.or(state.active_workspace);
    let active_host = active_workspace.and_then(|id| state.host_for_workspace(id));
    let pad = chrome::pad(width);
    let inner = chrome::inner_width(width);
    if let Some(snapshot) = active_host.and_then(|host_id| state.ports.get(&host_id)) {
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
            let forward_action = (!ambiguous)
                .then(|| active_workspace.zip(target.clone()))
                .flatten()
                .map(|(workspace_id, target)| MouseAction::ForwardTarget {
                    workspace_id,
                    target,
                });
            let process = listener.process.as_deref().unwrap_or("unknown");
            let (action, action_style, clickable) = if let Some(forward) = forward {
                match &forward.status {
                    crate::ports::ForwardStatus::Direct => (
                        format!("{} {}", glyphs.arrow(), forward.local_address.port()),
                        Style::default(),
                        forward_action.clone(),
                    ),
                    crate::ports::ForwardStatus::Active => (
                        format!("{} {}", glyphs.arrow(), forward.local_address.port()),
                        Style::default(),
                        forward_action.clone(),
                    ),
                    crate::ports::ForwardStatus::Reconnecting => (
                        "reconnecting".to_owned(),
                        warning_style(state),
                        forward_action.clone(),
                    ),
                    crate::ports::ForwardStatus::Cancelling => (
                        "cancelling".to_owned(),
                        section_style(state),
                        forward_action.clone(),
                    ),
                    crate::ports::ForwardStatus::PortConflict => (
                        "conflict".to_owned(),
                        danger_style(state),
                        forward_action.clone(),
                    ),
                    // The reason is fitted to the panel below, so it
                    // ellipsizes inside the column instead of clipping at the
                    // terminal edge.
                    crate::ports::ForwardStatus::Failed(reason) => (
                        format!("failed: {reason}"),
                        danger_style(state),
                        forward_action.clone(),
                    ),
                }
            } else if ambiguous {
                ("ambiguous".to_owned(), danger_style(state), None)
            } else if target.is_none() {
                ("invalid address".to_owned(), danger_style(state), None)
            } else {
                // Not the design's "enter to forward": Enter attaches the
                // selected workspace, so the row names the command that
                // actually runs, plus the click affordance that also works.
                (
                    ":forward · click".to_owned(),
                    section_style(state),
                    forward_action,
                )
            };

            let port = listener.port.to_string();
            let room = inner.saturating_sub(port.chars().count() + 1);
            let action = fit(glyphs, &action, room);
            let padding = inner
                .saturating_sub(port.chars().count() + Line::raw(&action).width())
                .max(1);
            lines.push(Line::from(vec![
                Span::raw(format!("{pad}{port}")),
                Span::raw(" ".repeat(padding)),
                Span::styled(action, action_style),
                Span::raw(pad.clone()),
            ]));
            targets.push(clickable);
            listener_rows += 1;
            // The detail row is never clickable; the port row above owns the
            // whole listener's hit target.
            lines.push(Line::styled(
                format!(
                    "{pad}{}",
                    fit(glyphs, &listener_detail(glyphs, listener, process), inner)
                ),
                section_style(state),
            ));
            targets.push(None);
        }
        // Unattributed listeners are a caveat on the list above, so they read
        // after it rather than pushing the ports themselves down.
        if let Some(warning) = &snapshot.warning {
            if listener_rows > 0 {
                lines.push(Line::raw(""));
                targets.push(None);
            }
            lines.push(Line::styled(
                format!(
                    "{pad}{} {}",
                    glyphs.warning(),
                    fit(glyphs, warning, inner.saturating_sub(2))
                ),
                warning_style(state),
            ));
            lines.push(Line::styled(
                format!("{pad}:ports --all-host"),
                section_style(state),
            ));
            targets.extend([
                None,
                Some(MouseAction::PrefillCommand(":ports --all-host".to_owned())),
            ]);
            all_host_hint_shown = true;
        }
    }
    if listener_rows == 0 && !all_host_hint_shown {
        lines.push(Line::raw(format!("{pad}No workspace ports found.")));
        lines.push(Line::styled(
            format!("{pad}:ports --all-host"),
            section_style(state),
        ));
        targets.extend([
            None,
            Some(MouseAction::PrefillCommand(":ports --all-host".to_owned())),
        ]);
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
