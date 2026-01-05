//! Layout utilities for rect manipulation.
//!
//! Pure functions for computing layout areas. No state mutation.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Inset a rect horizontally by padding on both sides.
pub fn inset_horizontal(area: Rect, padding: u16) -> Rect {
    if area.width <= padding * 2 {
        return area;
    }
    Rect {
        x: area.x + padding,
        width: area.width - padding * 2,
        ..area
    }
}

/// Create a centered rect with given percentage of parent dimensions.
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
