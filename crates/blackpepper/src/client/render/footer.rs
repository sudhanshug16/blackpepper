use super::style::{accent_badge_style, accent_style, panel_style, status_span, warning_style};
use crate::client::{ClientMode, ClientState, DisplayStatus};
use crate::ports::{ForwardStatus, ProbeCompleteness};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub(super) fn render_footer(state: &ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let line = if state.mode == ClientMode::Work {
        work_footer(state, area.width)
    } else if state.command_active {
        command_footer(state)
    } else {
        let mode = mode_badge(state);
        if let Some(output) = state.visible_output() {
            Line::from(vec![mode, Span::raw("  "), Span::raw(output.to_owned())])
        } else {
            Line::from(vec![mode, Span::raw(default_footer_hint(state))])
        }
    };
    frame.render_widget(Paragraph::new(line).style(panel_style(state)), area);
}

fn mode_badge(state: &ClientState) -> Span<'static> {
    let label = match state.mode {
        ClientMode::Work => "",
        ClientMode::Manage => " MANAGE ",
        ClientMode::Authenticate => " AUTHENTICATE ",
    };
    Span::styled(label, accent_badge_style(state))
}

fn command_footer(state: &ClientState) -> Line<'static> {
    let input = state.command_input.strip_prefix(':').unwrap_or_default();
    Line::from(vec![
        mode_badge(state),
        Span::raw("  "),
        Span::styled(":", accent_style(state)),
        Span::raw(input.to_owned()),
        Span::styled(
            " ",
            Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
        ),
        Span::raw("  · enter run · esc cancel"),
    ])
}

fn work_footer(state: &ClientState, width: u16) -> Line<'static> {
    let mut spans = vec![
        Span::styled("bp", accent_style(state)),
        Span::raw("  blackpepper"),
    ];
    if let Some(output) = state.visible_output() {
        spans.push(Span::raw("  ·  "));
        spans.push(Span::raw(output.to_owned()));
        return Line::from(spans);
    }
    if let Some(workspace_id) = state.active_workspace {
        let workspace = state
            .snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .and_then(|workspace| {
                workspace.display_name.clone().or_else(|| {
                    std::path::Path::new(&workspace.root_path)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                })
            })
            .unwrap_or_else(|| "workspace".to_owned());
        let status = state
            .statuses
            .get(&workspace_id)
            .copied()
            .unwrap_or(DisplayStatus::Idle);
        let manage = format!("  ·  {} manage", state.config.keymap.toggle_mode);
        let fixed_width = Line::from(spans.clone()).width()
            + 4
            + Line::raw(status.public_text()).width()
            + Line::raw(&manage).width();
        let workspace =
            truncate_to_width(&workspace, usize::from(width).saturating_sub(fixed_width));
        spans.push(Span::raw(format!("  {workspace}  ")));
        spans.push(status_span(state, status));
        spans.push(Span::raw(manage));
    } else {
        spans.push(Span::raw(format!(
            "  ·  {} manage",
            state.config.keymap.toggle_mode
        )));
    }
    let attention = work_attention(state);
    if !attention.is_empty() {
        push_if_fits(
            &mut spans,
            Span::styled(
                format!(" · {}", attention.join(" · ")),
                warning_style(state),
            ),
            width,
        );
    }
    push_if_fits(
        &mut spans,
        Span::raw(format!(" · {} next", state.config.keymap.switch_workspace)),
        width,
    );
    push_if_fits(
        &mut spans,
        Span::raw(format!(" · {} list", state.config.keymap.workspace_overlay)),
        width,
    );
    Line::from(spans)
}

fn push_if_fits(spans: &mut Vec<Span<'static>>, span: Span<'static>, width: u16) {
    let current = Line::from(spans.clone()).width();
    if current + span.width() <= usize::from(width) {
        spans.push(span);
    }
}

fn truncate_to_width(value: &str, width: usize) -> String {
    if Line::raw(value).width() <= width {
        return value.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut output = String::new();
    for character in value.chars() {
        let candidate = format!("{output}{character}…");
        if Line::raw(&candidate).width() > width {
            break;
        }
        output.push(character);
    }
    output.push('…');
    output
}

pub(super) fn work_attention(state: &ClientState) -> Vec<String> {
    let Some(workspace_id) = state.active_workspace else {
        return Vec::new();
    };
    let mut attention = Vec::new();
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
            "  {} manage · {} next · {} list",
            state.config.keymap.toggle_mode,
            state.config.keymap.switch_workspace,
            state.config.keymap.workspace_overlay,
        ),
        ClientMode::Manage if !state.host_operations.is_empty() => {
            "  esc cancel host work · : command · q quit".to_owned()
        }
        ClientMode::Manage => "  ↑↓ select · enter attach · : command · q quit".to_owned(),
        ClientMode::Authenticate => "  OpenSSH prompt · Ctrl+C cancel".to_owned(),
    }
}
