use super::chrome;
use super::glyph::Glyphs;
use super::style::{anchor_style, panel_style, section_style};
use crate::client::ClientState;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub(super) fn render_header(state: &ClientState, frame: &mut ratatui::Frame, area: Rect) {
    let glyphs = Glyphs::of(state);
    let pad = chrome::pad(area.width);
    // The repository segment sits with the version on the right, so the left
    // anchor never shifts as branches change. It sheds detail from the left as
    // the row narrows, and never emits a half-clipped branch or PR number.
    let right = right_group(state, chrome::inner_width(area.width));
    let context = header_context(state);
    let fixed_width = 2 * usize::from(chrome::gutter(area.width))
        + 2
        + 2
        + "blackpepper".len()
        + 2
        + right.chars().count();
    let context_width = usize::from(area.width).saturating_sub(fixed_width + 2);
    let context = truncate(glyphs, &context, context_width);
    let used = fixed_width + context.chars().count();
    let padding = usize::from(area.width).saturating_sub(used);
    let line = Line::from(vec![
        Span::raw(pad.clone()),
        Span::styled("bp", anchor_style(state)),
        Span::raw("  blackpepper  "),
        Span::styled(context, section_style(state)),
        Span::raw(" ".repeat(padding)),
        Span::styled(right, section_style(state)),
        Span::raw(pad),
    ]);
    frame.render_widget(Paragraph::new(line).style(panel_style(state)), area);
}

/// The widest repository summary that still leaves the workspace path room to
/// breathe, dropping whole facts rather than clipping one. The version is the
/// last thing standing because it is the only part that is always true.
fn right_group(state: &ClientState, inner_width: usize) -> String {
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let separator = Glyphs::of(state).separator();
    // A right group may claim at most half the row; the rest belongs to the
    // workspace path, which is what tells you where you are.
    let budget = inner_width / 2;
    let Some(parts) = repository_parts(state) else {
        return version;
    };
    let candidates = [
        format!(
            "{}{} {separator} {version}",
            parts.branch,
            parts.divergence_and_pull_request(separator)
        ),
        format!("{}{} {separator} {version}", parts.branch, parts.divergence),
        format!("{} {separator} {version}", parts.branch),
        version.clone(),
    ];
    candidates
        .into_iter()
        .find(|candidate| candidate.chars().count() <= budget)
        .unwrap_or(version)
}

/// The repository facts, kept separate so the row can drop whole ones.
struct RepositoryParts {
    /// Branch name plus the dirty marker.
    branch: String,
    divergence: String,
    pull_request: Option<String>,
}

impl RepositoryParts {
    fn divergence_and_pull_request(&self, separator: &str) -> String {
        match self.pull_request.as_ref() {
            Some(pull_request) => format!("{} {separator} {pull_request}", self.divergence),
            None => self.divergence.clone(),
        }
    }
}

/// Returns `None` when the host has not reported on this checkout, so an
/// unreachable host shows no branch rather than a stale one.
fn repository_parts(state: &ClientState) -> Option<RepositoryParts> {
    let glyphs = Glyphs::of(state);
    let workspace = state.selected_workspace.or(state.active_workspace)?;
    let overview = state.overviews.get(&workspace)?;
    let head = overview.head.as_deref()?;
    let dirty = if overview.dirty { "*" } else { "" };
    let mut divergence = String::new();
    if overview.ahead > 0 {
        divergence.push_str(&format!(" {}{}", glyphs.ahead(), overview.ahead));
    }
    if overview.behind > 0 {
        divergence.push_str(&format!(" {}{}", glyphs.behind(), overview.behind));
    }
    Some(RepositoryParts {
        branch: format!("{head}{dirty}"),
        divergence,
        pull_request: overview.pull_request.as_ref().map(|pull_request| {
            format!("PR #{} {}", pull_request.number, pull_request.state.word())
        }),
    })
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
