use super::style::{accent_style, panel_style, section_style};
use crate::client::ClientState;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub(super) fn render_header(state: &ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let context = header_context(state);
    let fixed_width = 2 + 2 + "blackpepper".len() + 2 + version.len();
    let context_width = usize::from(area.width).saturating_sub(fixed_width + 1);
    let context = truncate(&context, context_width);
    let used = fixed_width + context.chars().count();
    let padding = usize::from(area.width).saturating_sub(used);
    let line = Line::from(vec![
        Span::styled("bp", accent_style(state)),
        Span::raw("  blackpepper  "),
        Span::styled(context, section_style(state)),
        Span::raw(" ".repeat(padding)),
        Span::styled(version, section_style(state)),
    ]);
    frame.render_widget(Paragraph::new(line).style(panel_style(state)), area);
}

fn header_context(state: &ClientState) -> String {
    let workspace = state
        .selected_workspace
        .or(state.active_workspace)
        .and_then(|id| {
            state
                .snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.id == id)
        });
    let Some(workspace) = workspace else {
        return "no workspace".to_owned();
    };
    let host = state
        .snapshot
        .hosts
        .iter()
        .find(|host| host.id == workspace.host_id)
        .map(|host| host.display_name.as_str())
        .unwrap_or("unknown host");
    format!("{host}:{}", workspace.root_path)
}

fn truncate(value: &str, columns: usize) -> String {
    if value.chars().count() <= columns {
        return value.to_owned();
    }
    if columns == 0 {
        return String::new();
    }
    if columns == 1 {
        return "…".to_owned();
    }
    let mut output = value.chars().take(columns - 1).collect::<String>();
    output.push('…');
    output
}
