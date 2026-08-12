use super::style::{accent_style, section_style, ui_style, warning_style};
use crate::client::{ClientMode, ClientState};
use crate::core::RepositoryIdentity;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

pub(super) fn render_terminal(state: &mut ClientState, frame: &mut ratatui::Frame, area: Rect) {
    if state.mode == ClientMode::Authenticate {
        render_authentication(state, frame, area);
        return;
    }
    if state.mode == ClientMode::Manage {
        if state.pending_approval.is_some() {
            render_approval(state, frame, area);
            return;
        }
        if state.detail.is_some() {
            render_detail(state, frame, area);
            return;
        }
    }

    let body = if state.mode == ClientMode::Work || area.height <= 1 {
        area
    } else {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);
        render_session_label(state, frame, rows[0]);
        rows[1]
    };
    render_session_body(state, frame, body);
}

fn render_session_label(state: &ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let clients = state
        .active_workspace
        .and_then(|workspace| state.connected_clients.get(&workspace).copied())
        .map(|count| match count {
            1 => "zellij · 1 client".to_owned(),
            count => format!("zellij · {count} clients"),
        })
        .unwrap_or_else(|| "zellij".to_owned());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("SESSION", section_style(state)),
            Span::styled(format!("  {clients}"), section_style(state)),
        ]))
        .style(ui_style(state)),
        area,
    );
}

fn render_session_body(state: &mut ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let Some(active) = state.active_workspace else {
        state.terminal_area = Some(area);
        frame.render_widget(
            Paragraph::new(first_run_lines(state, area))
                .style(ui_style(state))
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    };
    let Some(terminal) = state.terminals.get_mut(&active) else {
        state.terminal_area = Some(area);
        frame.render_widget(
            Paragraph::new("Workspace is detached. Press Enter to attach.").style(ui_style(state)),
            area,
        );
        return;
    };
    state.terminal_area = Some(area);
    let resize_error = terminal.resize(area.height, area.width).err();
    let lines = terminal.render(area.height, area.width);
    if let Some(error) = resize_error {
        state.set_output(format!(
            "The attached terminal could not be resized; input remains available: {error}"
        ));
    }
    frame.render_widget(Paragraph::new(lines).style(ui_style(state)), area);
}

fn first_run_lines(state: &ClientState, area: Rect) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if area.width >= 12 && area.height >= 9 {
        lines.extend([
            Line::styled("█", accent_style(state).add_modifier(Modifier::BOLD)),
            Line::styled("█▀▄  █▀▄", accent_style(state).add_modifier(Modifier::BOLD)),
            Line::styled("█▄▀  █▄▀", accent_style(state).add_modifier(Modifier::BOLD)),
            Line::styled("     █", accent_style(state).add_modifier(Modifier::BOLD)),
            Line::raw(""),
        ]);
    }
    lines.push(Line::raw(format!(
        "blackpepper v{}",
        env!("CARGO_PKG_VERSION")
    )));
    lines.push(Line::raw(""));
    if let Some(workspace) = state
        .selected_workspace
        .and_then(|id| state.snapshot.workspaces.iter().find(|item| item.id == id))
    {
        lines.push(Line::raw(format!("{} is registered.", workspace.root_path)));
        lines.push(Line::raw("enter  open this workspace"));
    } else {
        lines.push(Line::raw("No workspaces registered."));
        lines.push(Line::raw(":workspace add <path>"));
    }
    lines.push(Line::raw(":host add <name> <alias>"));
    lines
}

fn render_approval(state: &mut ClientState, frame: &mut ratatui::Frame, area: Rect) {
    state.terminal_area = None;
    let pending = state
        .pending_approval
        .as_ref()
        .expect("approval checked before render");
    let repository = approval_repository(state, pending.workspace_id);
    let text = format!(
        "worktrunk will mutate this repository\n\nrepository\n{repository}\n\n{}",
        pending.review
    );
    render_focus_view(
        state,
        frame,
        area,
        Line::from(vec![
            Span::styled("⚠ ", warning_style(state)),
            Span::raw("APPROVAL"),
        ]),
        text,
        state.approval_scroll,
    );
}

fn approval_repository(state: &ClientState, workspace_id: crate::core::WorkspaceId) -> String {
    let Some(workspace) = state
        .snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.id == workspace_id)
    else {
        return "unknown repository".to_owned();
    };
    match workspace.repository.as_ref() {
        Some(RepositoryIdentity::Remote { canonical_url }) => canonical_url.clone(),
        Some(RepositoryIdentity::Local { git_common_dir, .. }) => git_common_dir.clone(),
        None => workspace.root_path.clone(),
    }
}

fn render_detail(state: &mut ClientState, frame: &mut ratatui::Frame, area: Rect) {
    state.terminal_area = None;
    let detail = state.detail.as_ref().expect("detail checked before render");
    render_focus_view(
        state,
        frame,
        area,
        Line::from(format!(
            "{}  Esc close · ↑↓ scroll",
            detail.title.to_uppercase()
        )),
        detail.body.clone(),
        state.detail_scroll,
    );
}

fn render_authentication(state: &mut ClientState, frame: &mut ratatui::Frame, area: Rect) {
    state.terminal_area = None;
    let output = String::from_utf8_lossy(&state.authentication_output);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);
    frame.render_widget(
        Paragraph::new("SSH AUTHENTICATION").style(ui_style(state).add_modifier(Modifier::BOLD)),
        rows[0],
    );
    let notice = "OpenSSH owns authentication.\nBlackpepper does not store credentials.";
    let mut notice_lines = wrap_terminal_text(notice, rows[1].width);
    let mut transcript = wrap_terminal_text(&output, rows[1].width);
    let reserve_for_prompt = usize::from(rows[1].height > 0 && !output.is_empty());
    notice_lines.truncate(usize::from(rows[1].height).saturating_sub(reserve_for_prompt));
    let spacer = usize::from(
        !notice_lines.is_empty()
            && !transcript.is_empty()
            && notice_lines.len() + reserve_for_prompt < usize::from(rows[1].height),
    );
    let transcript_rows = usize::from(rows[1].height)
        .saturating_sub(notice_lines.len())
        .saturating_sub(spacer);
    if transcript.len() > transcript_rows {
        transcript.drain(..transcript.len() - transcript_rows);
    }
    notice_lines.extend((0..spacer).map(|_| Line::raw("")));
    notice_lines.extend(transcript);
    frame.render_widget(Paragraph::new(notice_lines).style(ui_style(state)), rows[1]);
}

fn wrap_terminal_text(value: &str, width: u16) -> Vec<Line<'static>> {
    if width == 0 || value.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for character in value.chars() {
        if character == '\r' {
            continue;
        }
        if character == '\n' {
            lines.push(Line::raw(std::mem::take(&mut current)));
            continue;
        }
        let candidate = format!("{current}{character}");
        if !current.is_empty() && Line::raw(&candidate).width() > usize::from(width) {
            lines.push(Line::raw(std::mem::take(&mut current)));
        }
        current.push(character);
    }
    if !current.is_empty() || value.ends_with('\n') {
        lines.push(Line::raw(current));
    }
    lines
}

fn render_focus_view(
    state: &ClientState,
    frame: &mut ratatui::Frame,
    area: Rect,
    title: Line<'static>,
    body: String,
    scroll: u16,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);
    frame.render_widget(
        Paragraph::new(title).style(ui_style(state).add_modifier(Modifier::BOLD)),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(body)
            .style(ui_style(state))
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false }),
        rows[1],
    );
}
