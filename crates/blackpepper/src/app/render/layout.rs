use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::super::state::{App, Mode};
use crate::repo_status::{Divergence, PrErrorKind, PrState, PrStatus};

/// Render horizontal separator.
pub(super) fn render_separator(_app: &App, frame: &mut ratatui::Frame, area: Rect, width: usize) {
    let separator = Paragraph::new(Line::raw(dashed_line(width))).style(
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    );
    frame.render_widget(separator, area);
}

/// Render command bar (mode indicator or input).
pub(super) fn render_command_bar(app: &App, frame: &mut ratatui::Frame, area: Rect) {
    let workspace_name = app
        .active_workspace
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    if let Some(name) = workspace_name {
        let version = env!("CARGO_PKG_VERSION");
        let width = area.width as usize;
        let max_label_width = width.saturating_sub(2);
        if let Some(label) = build_status_label(app, name, version, max_label_width) {
            let label_len = label.len;
            if width > label_len + 1 {
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Min(1),
                        Constraint::Length((label_len + 1) as u16),
                    ])
                    .split(area);

                let command_line = command_line(app);
                frame.render_widget(Paragraph::new(command_line), chunks[0]);

                let label = Paragraph::new(Line::from(label.spans))
                    .alignment(Alignment::Right);
                frame.render_widget(label, chunks[1]);
                return;
            }
        }
    }

    let command_line = command_line(app);
    frame.render_widget(Paragraph::new(command_line), area);
}

/// Build the command line content (mode label or input).
fn command_line(app: &App) -> Line<'_> {
    if app.command_active {
        // Command input mode: show input with cursor
        Line::from(vec![
            Span::styled(app.command_input.clone(), Style::default().fg(Color::White)),
            Span::styled(" ", Style::default().bg(Color::White).fg(Color::Black)),
        ])
    } else {
        // Mode indicator
        let label = match app.mode {
            Mode::Work => "-- WORK --",
            Mode::Manage => "-- MANAGE --",
        };
        let style = if app.mode == Mode::Manage {
            Style::default().bg(Color::Magenta).fg(Color::Black)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        Line::from(vec![Span::styled(label, style)])
    }
}

struct StatusLabel {
    spans: Vec<Span<'static>>,
    len: usize,
}

struct Segment {
    text: String,
    spans: Vec<Span<'static>>,
}

fn build_status_label(
    app: &App,
    workspace: &str,
    version: &str,
    max_width: usize,
) -> Option<StatusLabel> {
    if max_width == 0 {
        return None;
    }
    let dim_style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::DIM);
    let name = app.repo_status.head.as_deref().unwrap_or(workspace);
    let version_segment = segment_plain(format!("v{version}"), dim_style);
    let branch_segment = fit_branch_segment(
        name,
        app.repo_status.dirty,
        &version_segment,
        max_width,
        dim_style,
    )?;
    let mut segments = vec![branch_segment, version_segment];

    if let Some(divergence) = app.repo_status.divergence.as_ref() {
        let segment = format_divergence_segment(divergence, false, dim_style);
        if can_prepend_segment(&segments, &segment, max_width) {
            segments.insert(0, segment);
        } else {
            let compact = format_divergence_segment(divergence, true, dim_style);
            if can_prepend_segment(&segments, &compact, max_width) {
                segments.insert(0, compact);
            }
        }
    }

    if let Some(max_len) = available_for_prepend(&segments, max_width) {
        if let Some(pr_segment) = format_pr_segment(&app.repo_status.pr, max_len, dim_style) {
            if can_prepend_segment(&segments, &pr_segment, max_width) {
                segments.insert(0, pr_segment);
            }
        }
    }

    Some(join_segments(&segments, dim_style))
}

fn fit_branch_segment(
    name: &str,
    dirty: bool,
    version: &Segment,
    max_width: usize,
    style: Style,
) -> Option<Segment> {
    let version_len = segment_len(version);
    let separator_len = 3;
    if max_width <= version_len + separator_len {
        return None;
    }
    let max_branch_len = max_width - (version_len + separator_len);
    if max_branch_len < 2 {
        return None;
    }
    let inner_len = max_branch_len.saturating_sub(2);
    let marker = if dirty { "*" } else { "" };
    let marker_len = marker.chars().count();
    let available = inner_len.saturating_sub(marker_len);
    let branch = if available == 0 {
        marker.to_string()
    } else if name.chars().count() > available {
        let mut truncated = truncate_label(name, available);
        truncated.push_str(marker);
        truncated
    } else {
        format!("{name}{marker}")
    };
    if dirty {
        let dirty_style = Style::default().fg(Color::Rgb(255, 140, 0));
        let mut spans = Vec::new();
        spans.push(Span::styled("(".to_string(), style));
        spans.push(Span::styled(branch.clone(), dirty_style));
        spans.push(Span::styled(")".to_string(), style));
        let text = format!("({branch})");
        Some(Segment { text, spans })
    } else {
        Some(segment_plain(format!("({branch})"), style))
    }
}

fn join_segments(segments: &[Segment], style: Style) -> StatusLabel {
    let mut spans = Vec::new();
    let mut len = 0;
    for (idx, segment) in segments.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled(" | ".to_string(), style));
            len += 3;
        }
        spans.extend(segment.spans.iter().cloned());
        len += segment_len(segment);
    }
    StatusLabel { spans, len }
}

fn segment_plain(text: String, style: Style) -> Segment {
    Segment {
        text: text.clone(),
        spans: vec![Span::styled(text, style)],
    }
}

fn segment_len(segment: &Segment) -> usize {
    segment.text.chars().count()
}

fn available_for_prepend(segments: &[Segment], max_width: usize) -> Option<usize> {
    let current_len = joined_len(segments);
    if current_len >= max_width {
        return None;
    }
    let separator_len = if segments.is_empty() { 0 } else { 3 };
    max_width.checked_sub(current_len + separator_len)
}

fn can_prepend_segment(segments: &[Segment], segment: &Segment, max_width: usize) -> bool {
    let segment_len = segment_len(segment);
    let separator_len = if segments.is_empty() { 0 } else { 3 };
    joined_len(segments) + separator_len + segment_len <= max_width
}

fn joined_len(segments: &[Segment]) -> usize {
    if segments.is_empty() {
        return 0;
    }
    let separator_len: usize = 3;
    let total: usize = segments.iter().map(segment_len).sum();
    total + separator_len.saturating_mul(segments.len().saturating_sub(1))
}

fn format_divergence_segment(
    divergence: &Divergence,
    compact: bool,
    style: Style,
) -> Segment {
    let mut parts = Vec::new();
    if divergence.ahead > 0 {
        parts.push(format!("↑{}", divergence.ahead));
    }
    if divergence.behind > 0 {
        parts.push(format!("↓{}", divergence.behind));
    }
    let text = if compact { parts.join("") } else { parts.join(" ") };
    segment_plain(text, style)
}

fn format_pr_segment(pr: &PrStatus, max_len: usize, style: Style) -> Option<Segment> {
    if max_len == 0 {
        return None;
    }
    match pr {
        PrStatus::None => {
            let label = "PR: none";
            let text = if label.chars().count() <= max_len {
                label.to_string()
            } else if max_len >= 2 {
                truncate_label("PR", max_len)
            } else {
                return None;
            };
            Some(segment_plain(text, style))
        }
        PrStatus::Error(error) => {
            let message = match error.kind {
                PrErrorKind::MissingCli => "gh cli not available".to_string(),
                PrErrorKind::Other => error.message.trim().to_string(),
            };
            let prefix = "⚠️ ";
            let prefix_len = prefix.chars().count();
            if prefix_len > max_len {
                return Some(segment_plain(truncate_label("⚠️", max_len), style));
            }
            let available = max_len.saturating_sub(prefix_len);
            let message = if message.chars().count() > available {
                truncate_label(&message, available)
            } else {
                message
            };
            let text = format!("{prefix}{message}");
            Some(segment_plain(text, style))
        }
        PrStatus::Info(info) => {
            let status = match info.state {
                PrState::Open => ("open", Color::Green),
                PrState::Closed => ("closed", Color::Red),
                PrState::Merged => ("merged", Color::Magenta),
                PrState::Draft => ("draft", Color::Gray),
            };
            let prefix_base = format!("PR #{}", info.number);
            let suffix = format!(" ({})", status.0);
            let base_len = prefix_base.chars().count() + suffix.chars().count();
            if base_len > max_len {
                let compact = format!("PR #{} ({})", info.number, status.0);
                if compact.chars().count() <= max_len {
                    return Some(segment_plain(compact, style));
                }
                let short = format!("PR #{}", info.number);
                if short.chars().count() <= max_len {
                    return Some(segment_plain(short, style));
                }
                return None;
            }
            let title = info.title.trim();
            let available_title = max_len.saturating_sub(base_len);
            let (prefix, title) = if title.is_empty() || available_title <= 1 {
                (prefix_base, String::new())
            } else {
                let max_title = available_title.saturating_sub(1);
                (format!("{prefix_base} "), truncate_label(title, max_title))
            };
            let open = " (".to_string();
            let close = ")".to_string();
            let mut spans = Vec::new();
            spans.push(Span::styled(prefix.clone(), style));
            if !title.is_empty() {
                spans.push(Span::styled(title.clone(), style));
            }
            spans.push(Span::styled(open.clone(), style));
            spans.push(Span::styled(
                status.0.to_string(),
                Style::default().fg(status.1),
            ));
            spans.push(Span::styled(close.clone(), style));
            let text = format!("{prefix}{title}{open}{}{close}", status.0);
            let text = if text.chars().count() <= max_len {
                text
            } else {
                let compact = format!("PR #{} ({})", info.number, status.0);
                if compact.chars().count() <= max_len {
                    return Some(segment_plain(compact, style));
                }
                format!("PR #{}", info.number)
            };
            Some(Segment { text, spans })
        }
    }
}

fn truncate_label(label: &str, max_len: usize) -> String {
    let len = label.chars().count();
    if len <= max_len {
        return label.to_string();
    }
    if max_len <= 3 {
        return label.chars().take(max_len).collect();
    }
    let keep = max_len - 3;
    let mut out: String = label.chars().take(keep).collect();
    out.push_str("...");
    out
}

/// Dashed separator line.
fn dashed_line(width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let pattern = "- ";
    pattern.repeat(width / pattern.len() + 1)[..width].to_string()
}

/// Inset a rect horizontally by padding on each side.
pub(super) fn inset_horizontal(area: Rect, padding: u16) -> Rect {
    if area.width <= padding * 2 {
        return area;
    }
    Rect {
        x: area.x + padding,
        width: area.width - padding * 2,
        ..area
    }
}

/// Create a centered rect with given percentage of parent.
pub(super) fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_width = r.width * percent_x / 100;
    let popup_height = r.height * percent_y / 100;
    let x = r.x + (r.width.saturating_sub(popup_width)) / 2;
    let y = r.y + (r.height.saturating_sub(popup_height)) / 2;
    Rect::new(x, y, popup_width, popup_height)
}
