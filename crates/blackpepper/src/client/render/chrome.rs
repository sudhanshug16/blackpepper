//! The frame rule every surface shares: a horizontal gutter, and one way to
//! push a segment to the right edge.
//!
//! The design insets every panel and status row by `2ch`. Four of forty columns
//! is too much at the compact floor, and the design's own narrow frames use a
//! single column there, so the gutter steps down with the surface rather than
//! staying flat.

use ratatui::layout::Rect;
use ratatui::text::Line;

/// Columns of inset on each side of a surface `width` columns wide. The
/// design's own panels are 30 and 32 columns and carry the full inset, so the
/// step down only applies to surfaces narrower than those.
pub(super) fn gutter(width: u16) -> u16 {
    match width {
        0..=11 => 0,
        12..=23 => 1,
        _ => 2,
    }
}

/// The area inside the gutter. Use for wrapped bodies drawn on the canvas,
/// where nothing needs to bleed into the gutter cells.
pub(super) fn inner(area: Rect) -> Rect {
    let gutter = gutter(area.width);
    Rect::new(
        area.x.saturating_add(gutter),
        area.y,
        area.width.saturating_sub(gutter.saturating_mul(2)),
        area.height,
    )
}

/// Usable columns once both gutters are removed. Use for raised panels, which
/// keep drawing across the full rect so the panel background still paints the
/// gutter cells, and instead prefix each line by hand.
pub(super) fn inner_width(width: u16) -> usize {
    usize::from(width.saturating_sub(gutter(width).saturating_mul(2)))
}

/// A `g`-column prefix string for one line of a raised panel.
pub(super) fn pad(width: u16) -> String {
    " ".repeat(usize::from(gutter(width)))
}

/// `left`, then enough spaces to land `right` against the inner right edge.
/// Never closes below a two-column gap, matching the design's `gap:2ch`; when
/// the pair cannot fit at all the gap collapses to that minimum and the caller's
/// own truncation absorbs the rest.
pub(super) fn right_aligned(left: &str, right: &str, inner_width: usize) -> String {
    let used = Line::raw(left).width() + Line::raw(right).width();
    let spacing = inner_width.saturating_sub(used).max(2);
    format!("{left}{}{right}", " ".repeat(spacing))
}

#[cfg(test)]
mod tests {
    use super::{gutter, inner_width, right_aligned};

    #[test]
    fn the_designs_own_panel_widths_keep_the_full_inset() {
        assert_eq!(gutter(120), 2);
        assert_eq!(gutter(32), 2, "the sidebar is 32 columns in the design");
        assert_eq!(gutter(30), 2, "the ports panel is 30 columns in the design");
        assert_eq!(gutter(24), 2);
        assert_eq!(gutter(23), 1);
        assert_eq!(gutter(12), 1);
        assert_eq!(gutter(11), 0);
        assert_eq!(gutter(0), 0);
    }

    #[test]
    fn inner_width_removes_both_gutters() {
        assert_eq!(inner_width(120), 116);
        assert_eq!(inner_width(32), 28);
        assert_eq!(inner_width(30), 26);
        assert_eq!(inner_width(10), 10);
    }

    #[test]
    fn right_alignment_lands_on_the_inner_edge_and_keeps_a_two_column_gap() {
        assert_eq!(
            right_aligned("PORTS", "1/2", 26),
            "PORTS                  1/2"
        );
        assert_eq!(right_aligned("PORTS", "1/2", 26).chars().count(), 26);
        // Too narrow to align: the gap floors at two rather than vanishing.
        assert_eq!(right_aligned("PORTS", "1/2", 6), "PORTS  1/2");
    }
}
