//! UI rendering methods.
//!
//! Handles all drawing/rendering for the TUI:
//! - Main layout (work area, separator, output, command bar)
//! - Overlays (workspace picker, loader)
//! - Terminal content rendering

mod layout;
mod output;
mod overlays;
mod work_area;

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;

use super::state::{App, BOTTOM_HORIZONTAL_PADDING};

/// Main render entry point. Called each frame by the event loop.
pub fn render(app: &mut App, frame: &mut ratatui::Frame) {
    let area = frame.area();
    let output_width =
        area.width
            .saturating_sub(BOTTOM_HORIZONTAL_PADDING.saturating_mul(2)) as usize;
    let output_lines = output::output_lines_owned(app, output_width);
    let output_height = output_lines.len() as u16;
    let separator_height = 1u16;
    let command_height = 1u16;

    // Vertical layout: work area | separator | output | command bar
    let split_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(separator_height),
            Constraint::Length(output_height),
            Constraint::Length(command_height),
        ])
        .split(area);

    work_area::render_work_area(app, frame, split_chunks[0]);
    layout::render_separator(app, frame, split_chunks[1], area.width as usize);

    if output_height > 0 {
        let output_area = layout::inset_horizontal(split_chunks[2], BOTTOM_HORIZONTAL_PADDING);
        let output = Paragraph::new(output_lines).style(Style::default().fg(Color::Gray));
        frame.render_widget(output, output_area);
    }

    let command_area = layout::inset_horizontal(split_chunks[3], BOTTOM_HORIZONTAL_PADDING);
    layout::render_command_bar(app, frame, command_area);

    // Render overlays on top if visible
    if app.overlay.visible {
        overlays::render_overlay(app, frame, area);
    }
    if app.prompt_overlay.visible {
        overlays::render_prompt_overlay(app, frame, area);
    }
    if app.command_overlay.visible {
        overlays::render_command_overlay(app, frame, area);
    } else if let Some(message) = &app.loading {
        overlays::render_loader(frame, area, message);
    }
}
