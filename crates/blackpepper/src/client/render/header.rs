use super::glyph::Glyphs;
use super::style::{anchor_style, panel_style, section_style};
use crate::client::ClientState;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub(super) fn render_header(state: &ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let glyphs = Glyphs::of(state);
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    // The repository segment sits with the version on the right, so the left
    // anchor never shifts as branches change.
    let right = match repository_summary(state) {
        Some(summary) => format!("{summary} {} {version}", glyphs.separator()),
        None => version,
    };
    let context = header_context(state);
    let fixed_width = 2 + 2 + "blackpepper".len() + 2 + right.chars().count();
    let context_width = usize::from(area.width).saturating_sub(fixed_width + 1);
    let context = truncate(glyphs, &context, context_width);
    let used = fixed_width + context.chars().count();
    let padding = usize::from(area.width).saturating_sub(used);
    let line = Line::from(vec![
        Span::styled("bp", anchor_style(state)),
        Span::raw("  blackpepper  "),
        Span::styled(context, section_style(state)),
        Span::raw(" ".repeat(padding)),
        Span::styled(right, section_style(state)),
    ]);
    frame.render_widget(Paragraph::new(line).style(panel_style(state)), area);
}

/// `main* ↑2 · PR #418 open`, omitting whichever parts do not apply. Returns
/// `None` when the host has not reported on this checkout, so an unreachable
/// host shows no branch rather than a stale one.
fn repository_summary(state: &ClientState) -> Option<String> {
    let glyphs = Glyphs::of(state);
    let workspace = state.selected_workspace.or(state.active_workspace)?;
    let overview = state.overviews.get(&workspace)?;
    let head = overview.head.as_deref()?;
    let dirty = if overview.dirty { "*" } else { "" };
    let mut summary = format!("{head}{dirty}");
    if overview.ahead > 0 {
        summary.push_str(&format!(" {}{}", glyphs.ahead(), overview.ahead));
    }
    if overview.behind > 0 {
        summary.push_str(&format!(" {}{}", glyphs.behind(), overview.behind));
    }
    if let Some(pull_request) = overview.pull_request.as_ref() {
        summary.push_str(&format!(
            " {} PR #{} {}",
            glyphs.separator(),
            pull_request.number,
            pull_request.state.word()
        ));
    }
    Some(summary)
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

fn truncate(glyphs: Glyphs, value: &str, columns: usize) -> String {
    if value.chars().count() <= columns {
        return value.to_owned();
    }
    if columns == 0 {
        return String::new();
    }
    let ellipsis = glyphs.ellipsis();
    if columns == 1 {
        return ellipsis.to_owned();
    }
    let mut output = value.chars().take(columns - 1).collect::<String>();
    output.push_str(ellipsis);
    output
}
