//! The views that take over the session column: approval, detail, and SSH
//! authentication.
//!
//! Each is a focused reading surface rather than a panel — they draw on the
//! canvas, inside the shared gutter, and own the whole column while open.

use super::super::chrome;
use super::super::glyph::Glyphs;
use super::super::style::{
    accent_badge_style, accent_style, mid_style, section_style, ui_style, warning_style,
};
use crate::client::ClientState;
use crate::core::RepositoryIdentity;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

/// The approval review. The warning line is the title — a separate APPROVAL
/// banner above it would say the same thing twice on a surface that has to
/// hold an exact argv. This is the one accent site outside the four the design
/// names, matching the design's own approval panel: it is the only
/// irreversible control in the client.
pub(in crate::client::render) fn render_approval(
    state: &mut ClientState,
    frame: &mut ratatui::Frame,
    area: Rect,
) {
    state.terminal_area = None;
    let glyphs = Glyphs::of(state);
    let pending = state
        .pending_approval
        .as_ref()
        .expect("approval checked before render");
    let repository = approval_repository(state, pending.workspace_id);
    let body = chrome::inner(area);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(glyphs.warning().to_owned(), warning_style(state)),
            Span::raw("  worktrunk will mutate this repository"),
        ]),
        Line::raw(""),
        Line::styled("repository", section_style(state)),
        Line::styled(repository, mid_style(state)),
        Line::raw(""),
    ];
    lines.extend(review_lines(state, &pending.review));
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(" :approve ", accent_badge_style(state)),
        Span::raw("  "),
        Span::styled(
            format!(
                "run {sep} esc dismiss {sep} {} scroll",
                glyphs.updown(),
                sep = glyphs.separator()
            ),
            section_style(state),
        ),
    ]));
    frame.render_widget(
        Paragraph::new(lines)
            .style(ui_style(state))
            .scroll((state.approval_scroll, 0))
            .wrap(Wrap { trim: false }),
        body,
    );
}

/// Style the review body: its field labels are dim, its values ink, so the
/// exact command stands out from the scaffolding around it.
fn review_lines(state: &ClientState, review: &str) -> Vec<Line<'static>> {
    const LABELS: [&str; 4] = [
        "mutation",
        "unapproved project hooks",
        "project hooks",
        "approval binds to this exact Worktrunk command and project hook plan.",
    ];
    review
        .lines()
        .map(|line| {
            if line.is_empty() {
                Line::raw("")
            } else if LABELS.contains(&line.trim()) || line.starts_with("approval binds") {
                Line::styled(line.to_owned(), section_style(state))
            } else {
                Line::raw(line.to_owned())
            }
        })
        .collect()
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

pub(in crate::client::render) fn render_detail(
    state: &mut ClientState,
    frame: &mut ratatui::Frame,
    area: Rect,
) {
    state.terminal_area = None;
    let detail = state.detail.as_ref().expect("detail checked before render");
    render_focus_view(
        state,
        frame,
        area,
        Line::from(format!(
            "{}  Esc close {} {} scroll",
            detail.title.to_uppercase(),
            Glyphs::of(state).separator(),
            Glyphs::of(state).updown()
        )),
        detail.body.clone(),
        state.detail_scroll,
    );
}

pub(in crate::client::render) fn render_authentication(
    state: &mut ClientState,
    frame: &mut ratatui::Frame,
    area: Rect,
) {
    state.terminal_area = None;
    let glyphs = Glyphs::of(state);
    let output = String::from_utf8_lossy(&state.authentication_output);
    let body = chrome::inner(area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(body);
    frame.render_widget(
        Paragraph::new("SSH AUTHENTICATION").style(ui_style(state).add_modifier(Modifier::BOLD)),
        rows[0],
    );
    // One dim line of ownership, and one dim line of escape, bracketing the
    // relayed OpenSSH transcript. Both are budgeted before the transcript is
    // trimmed so neither can be pushed off the pane.
    // Wrapped by hand: the transcript below must not be re-flowed, so the
    // paragraph cannot carry a global `Wrap` to fold these two for us.
    let notice = styled_wrap(
        "OpenSSH owns authentication. Blackpepper stores no credentials.",
        rows[1].width,
        section_style(state),
    );
    let closing = styled_wrap(
        &format!("^c cancels {} nothing is stored", glyphs.separator()),
        rows[1].width,
        section_style(state),
    );
    let mut transcript = wrap_terminal_text(&output, rows[1].width);
    // The last transcript line is the live prompt: accent it and park the
    // caret after it, which is the only cue OpenSSH gives us to render.
    if let Some(last) = transcript.pop() {
        let prompt = last
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        transcript.push(Line::from(vec![
            Span::styled(prompt, accent_style(state)),
            Span::styled(
                " ",
                ratatui::style::Style::default().add_modifier(Modifier::REVERSED),
            ),
        ]));
    }
    // Two passes. First every part gets one row, prompt first, so a pane too
    // short for the preamble still shows what OpenSSH is asking. Then the
    // fixed-size prose is completed before the transcript is allowed to grow,
    // because a half-stated security claim is worse than a shorter tail.
    let height = usize::from(rows[1].height);
    let wanted = [transcript.len(), notice.len(), closing.len()];
    let mut rows_for = [0usize; 3];
    let mut used = 0;
    for (pass, order) in [(1usize, [0, 1, 2]), (usize::MAX, [1, 2, 0])] {
        for index in order {
            let extra = wanted[index]
                .saturating_sub(rows_for[index])
                .min(pass)
                .min(height - used);
            rows_for[index] += extra;
            used += extra;
        }
    }
    let [transcript_rows, notice_rows, closing_rows] = rows_for;
    if transcript.len() > transcript_rows {
        transcript.drain(..transcript.len() - transcript_rows);
    }
    let mut lines: Vec<Line<'static>> = notice.into_iter().take(notice_rows).collect();
    if !transcript.is_empty() {
        if lines.len() + transcript.len() + closing_rows < height {
            lines.push(Line::raw(""));
        }
        lines.extend(transcript);
    }
    if closing_rows > 0 {
        if lines.len() + closing_rows < height {
            lines.push(Line::raw(""));
        }
        lines.extend(closing.into_iter().take(closing_rows));
    }
    frame.render_widget(Paragraph::new(lines).style(ui_style(state)), rows[1]);
}

/// Word-wrap our own prose to `width` and style every resulting row the same
/// way. Distinct from `wrap_terminal_text`, which wraps by column because it
/// is relaying OpenSSH output that must not be re-flowed.
fn styled_wrap(value: &str, width: u16, style: ratatui::style::Style) -> Vec<Line<'static>> {
    let width = usize::from(width);
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in value.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if Line::raw(&candidate).width() > width && !current.is_empty() {
            lines.push(Line::styled(std::mem::take(&mut current), style));
            current = word.to_owned();
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(Line::styled(current, style));
    }
    lines
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
        .split(chrome::inner(area));
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
