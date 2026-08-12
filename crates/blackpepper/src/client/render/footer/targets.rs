//! Hit targets for the action hints rendered in the Manage footer.

use crate::client::state::{MouseAction, MouseTarget};
use crate::client::ClientState;
use ratatui::layout::Rect;
use ratatui::text::Line;

pub(super) fn register_hint_targets(
    state: &mut ClientState,
    area: Rect,
    offset: usize,
    hint: &str,
) {
    let actions = if state.host_operations.is_empty() {
        [
            ("enter attach", MouseAction::AttachSelected),
            (": command", MouseAction::OpenCommand),
            ("q quit", MouseAction::Quit),
        ]
        .into_iter()
        .collect::<Vec<_>>()
    } else {
        [
            ("esc cancel host work", MouseAction::CancelHostOperation),
            (": command", MouseAction::OpenCommand),
            ("q quit", MouseAction::Quit),
        ]
        .into_iter()
        .collect::<Vec<_>>()
    };
    for (label, action) in actions {
        let Some(index) = hint.find(label) else {
            continue;
        };
        let start = offset + Line::raw(&hint[..index]).width();
        let width = Line::raw(label).width();
        if start >= usize::from(area.width) {
            continue;
        }
        state.mouse_targets.push(MouseTarget {
            area: Rect::new(
                area.x.saturating_add(start as u16),
                area.y,
                width.min(usize::from(area.width) - start) as u16,
                1,
            ),
            action,
        });
    }
}
