use super::{actions, ClientRuntime, ClientState};
use termwiz::input::{Modifiers, MouseButtons, MouseEvent};

pub(in crate::client::control) fn handle(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    mouse: MouseEvent,
) {
    if mouse.modifiers == Modifiers::NONE
        && mouse.mouse_buttons.contains(MouseButtons::VERT_WHEEL)
        && state.ports_area.is_some_and(|area| {
            let x = mouse.x.saturating_sub(1);
            let y = mouse.y.saturating_sub(1);
            x >= area.x
                && x < area.x.saturating_add(area.width)
                && y >= area.y
                && y < area.y.saturating_add(area.height)
        })
    {
        state.ports_scroll = if mouse.mouse_buttons.contains(MouseButtons::WHEEL_POSITIVE) {
            state.ports_scroll.saturating_sub(3)
        } else {
            state.ports_scroll.saturating_add(3)
        };
        return;
    }
    if !mouse.mouse_buttons.contains(MouseButtons::LEFT) || mouse.modifiers != Modifiers::NONE {
        return;
    }
    let x = mouse.x.saturating_sub(1);
    let y = mouse.y.saturating_sub(1);
    let Some(target) = state
        .port_click_targets
        .iter()
        .find(|target| target.contains(x, y))
        .cloned()
    else {
        return;
    };
    if let Some(forward) = state.forwards.iter().find(|forward| {
        forward.workspace_id == target.workspace_id && forward.target() == target.target
    }) {
        state.set_output(actions::existing_forward_message(forward));
        return;
    }
    if let Err(error) =
        actions::start_forward_target(state, runtime, target.workspace_id, target.target)
    {
        state.set_output(error);
    }
}
