use super::glyph::Glyphs;
use super::style::{
    accent_badge_style, anchor_style, panel_style, status_span, status_text, warning_style,
};
use crate::client::{ClientMode, ClientState, DisplayStatus};
use crate::ports::{ForwardStatus, ProbeCompleteness};
use ratatui::layout::Rect;
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
            // Anything wanting a person goes hard right, opposite the badge, so
            // the two ends of the row answer "where am I" and "what needs me".
            let mut spans = vec![mode, Span::raw(default_footer_hint(state))];
            if let Some(attention) = manage_attention(state) {
                push_right_aligned(&mut spans, attention, warning_style(state), area.width);
            }
            Line::from(spans)
        }
    };
    frame.render_widget(Paragraph::new(line).style(panel_style(state)), area);
}

/// The workspaces asking for a person, named so the badge is actionable
/// without first opening the sidebar.
fn manage_attention(state: &ClientState) -> Option<String> {
    let glyphs = Glyphs::of(state);
    let asking = state
        .tree
        .iter()
        .flat_map(|host| &host.repositories)
        .flat_map(|repository| &repository.workspaces)
        .filter(|workspace| workspace.status == DisplayStatus::NeedsInput)
        .map(|workspace| workspace.label.clone())
        .collect::<Vec<_>>();
    match asking.as_slice() {
        [] => None,
        [one] => Some(format!("{} {one} asks", glyphs.attention())),
        many => Some(format!("{} {} ask", glyphs.attention(), many.len())),
    }
}

/// Push `text` so its last column lands on the row's right edge. Dropped
/// entirely when the row is already full, because a partially drawn warning is
/// worse than none.
fn push_right_aligned(
    spans: &mut Vec<Span<'static>>,
    text: String,
    style: ratatui::style::Style,
    width: u16,
) {
    let used = Line::from(spans.clone()).width();
    let needed = Line::raw(&text).width();
    let Some(padding) = usize::from(width).checked_sub(used + needed + 1) else {
        return;
    };
    spans.push(Span::raw(" ".repeat(padding + 1)));
    spans.push(Span::styled(text, style));
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
    let mut spans = vec![mode_badge(state), Span::raw("  ")];
    spans.extend(super::command::command_line(state).spans);
    Line::from(spans)
}

fn work_footer(state: &ClientState, width: u16) -> Line<'static> {
    let glyphs = Glyphs::of(state);
    let separator = glyphs.separator();
    let mut spans = vec![
        Span::styled("bp", anchor_style(state)),
        Span::raw("  blackpepper"),
    ];
    if let Some(output) = state.visible_output() {
        spans.push(Span::raw(format!("  {separator}  ")));
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
        let detail = state.status_detail(workspace_id, status);
        let status_text = status_text(state, status, detail.as_deref());
        // The workspace name is the only elastic segment. Reserve the shortest
        // hint the row can end with, so a long name truncates instead of
        // pushing the Manage chord off the edge.
        let reserved = 2 + Line::raw(manage_hint(state)).width();
        let fixed_width =
            Line::from(spans.clone()).width() + 4 + Line::raw(&status_text).width() + reserved;
        let workspace = truncate_to_width(
            glyphs,
            &workspace,
            usize::from(width).saturating_sub(fixed_width),
        );
        spans.push(Span::raw(format!("  {workspace}  ")));
        spans.push(status_span(state, status, detail.as_deref()));
    }
    let attention = work_attention(state);
    if !attention.is_empty() {
        push_if_fits(
            &mut spans,
            Span::styled(
                format!(" {separator} {}", attention.join(&format!(" {separator} "))),
                warning_style(state),
            ),
            width,
        );
    }
    // Key hints go hard right on both status rows, so the left anchor stays put
    // and the eye never re-hunts for the mode or the workspace. Longer hint
    // sets are dropped first; the Manage chord is the one that always fits.
    let mut hints = vec![manage_hint(state)];
    if state.active_workspace.is_some() {
        hints.push(format!("{} next", state.config.keymap.switch_workspace));
        hints.push(format!("{} list", state.config.keymap.workspace_overlay));
    }
    for count in (1..=hints.len()).rev() {
        let text = hints[..count].join(&format!(" {separator} "));
        let used = Line::from(spans.clone()).width();
        if let Some(padding) = usize::from(width).checked_sub(used + Line::raw(&text).width() + 1) {
            spans.push(Span::raw(" ".repeat(padding + 1)));
            spans.push(Span::raw(text));
            break;
        }
    }
    Line::from(spans)
}

/// The one hint that must survive at any width: how to get back to Manage.
fn manage_hint(state: &ClientState) -> String {
    format!("{} manage", state.config.keymap.toggle_mode)
}

fn push_if_fits(spans: &mut Vec<Span<'static>>, span: Span<'static>, width: u16) {
    let current = Line::from(spans.clone()).width();
    if current + span.width() <= usize::from(width) {
        spans.push(span);
    }
}

fn truncate_to_width(glyphs: Glyphs, value: &str, width: usize) -> String {
    if Line::raw(value).width() <= width {
        return value.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let ellipsis = glyphs.ellipsis();
    let mut output = String::new();
    for character in value.chars() {
        let candidate = format!("{output}{character}{ellipsis}");
        if Line::raw(&candidate).width() > width {
            break;
        }
        output.push(character);
    }
    output.push_str(ellipsis);
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
    let glyphs = Glyphs::of(state);
    let separator = glyphs.separator();
    match state.mode {
        ClientMode::Work => format!(
            "  {} manage {separator} {} next {separator} {} list",
            state.config.keymap.toggle_mode,
            state.config.keymap.switch_workspace,
            state.config.keymap.workspace_overlay,
        ),
        ClientMode::Manage if !state.host_operations.is_empty() => {
            format!("  esc cancel host work {separator} : command {separator} q quit")
        }
        ClientMode::Manage => format!(
            "  {} select {separator} enter attach {separator} : command {separator} q quit",
            glyphs.updown()
        ),
        ClientMode::Authenticate => format!("  OpenSSH prompt {separator} Ctrl+C cancel"),
    }
}
