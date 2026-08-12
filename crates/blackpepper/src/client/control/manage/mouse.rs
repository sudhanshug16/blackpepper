use super::{actions, attach_selected, cancel_host_operation, modal, ClientRuntime, ClientState};
use crate::client::command::ClientCommand;
use crate::client::state::MouseAction;
use termwiz::input::{Modifiers, MouseButtons, MouseEvent};

pub(in crate::client::control) fn handle(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    mouse: MouseEvent,
) {
    if mouse.modifiers != Modifiers::NONE {
        return;
    }
    let x = mouse.x.saturating_sub(1);
    let y = mouse.y.saturating_sub(1);

    if mouse.mouse_buttons.contains(MouseButtons::VERT_WHEEL) {
        let Some(action) = state
            .mouse_targets
            .iter()
            .rev()
            .find(|target| target.contains(x, y) && target.action.is_scroll_target())
            .map(|target| target.action.clone())
        else {
            return;
        };
        let direction = if mouse.mouse_buttons.contains(MouseButtons::WHEEL_POSITIVE) {
            -1
        } else {
            1
        };
        scroll(state, action, direction);
        return;
    }

    if !mouse.mouse_buttons.contains(MouseButtons::LEFT) {
        return;
    }
    let Some(action) = state
        .mouse_targets
        .iter()
        .rev()
        .find(|target| target.contains(x, y))
        .map(|target| target.action.clone())
    else {
        return;
    };
    click(state, runtime, action);
}

fn click(state: &mut ClientState, runtime: &mut ClientRuntime, action: MouseAction) {
    match action {
        MouseAction::ScrollSidebar
        | MouseAction::ScrollPicker
        | MouseAction::ScrollHelp
        | MouseAction::ScrollDetail
        | MouseAction::ScrollApproval
        | MouseAction::ScrollPorts => {}
        MouseAction::SelectHost(host_id) => {
            state.selected_host = Some(host_id);
            state.selected_workspace = state
                .tree
                .iter()
                .find(|host| host.id == host_id)
                .and_then(|host| {
                    host.repositories
                        .iter()
                        .flat_map(|repo| &repo.workspaces)
                        .next()
                })
                .map(|workspace| workspace.id);
        }
        MouseAction::SelectWorkspace(workspace_id) => {
            state.selected_workspace = Some(workspace_id);
            state.selected_host = state.host_for_workspace(workspace_id);
        }
        MouseAction::AttachSelected => attach_selected(state, runtime),
        MouseAction::AttachNext => {
            state.select_next(1);
            attach_selected(state, runtime);
        }
        MouseAction::EnterWork => {
            let Some(workspace_id) = state.active_workspace else {
                return;
            };
            if !state.terminals.contains_key(&workspace_id) {
                state.set_output("The workspace is detached; click its session area to attach.");
                return;
            }
            state.mark_workspace_completions_seen(workspace_id);
            state.mode = crate::client::ClientMode::Work;
        }
        MouseAction::EnterManage => state.mode = crate::client::ClientMode::Manage,
        MouseAction::OpenPicker => state.open_picker(),
        MouseAction::OpenCommand => modal::open_command(state),
        MouseAction::CloseCommand => modal::close_command(state),
        MouseAction::PrefillCommand(input) => {
            state.help = None;
            state.picker = None;
            modal::prefill_command(state, input);
        }
        MouseAction::ChooseCompletion(index) => modal::choose_completion(state, index),
        MouseAction::ChoosePicker(workspace_id) => {
            state.picker = None;
            state.selected_workspace = Some(workspace_id);
            state.selected_host = state.host_for_workspace(workspace_id);
            attach_selected(state, runtime);
        }
        MouseAction::ClosePicker => state.picker = None,
        MouseAction::CloseHelp => state.help = None,
        MouseAction::CloseDetail => {
            state.close_detail();
        }
        MouseAction::Approve => {
            actions::execute_command(state, runtime, ClientCommand::Approve);
        }
        MouseAction::DismissApproval => {
            state.pending_approval = None;
            state.approval_scroll = 0;
            state.set_output("Approval dismissed; no Worktrunk mutation ran.");
        }
        MouseAction::Quit => state.should_quit = true,
        MouseAction::CancelHostOperation => {
            cancel_host_operation(state, runtime);
        }
        MouseAction::ForwardTarget {
            workspace_id,
            target,
        } => {
            if let Some(forward) = state
                .forwards
                .iter()
                .find(|forward| forward.workspace_id == workspace_id && forward.target() == target)
            {
                state.set_output(actions::existing_forward_message(forward));
            } else if let Err(error) =
                actions::start_forward_target(state, runtime, workspace_id, target)
            {
                state.set_output(error);
            }
        }
    }
}

fn scroll(state: &mut ClientState, action: MouseAction, direction: i32) {
    let amount = 3_u16;
    let update = |value: &mut u16| {
        *value = if direction < 0 {
            value.saturating_sub(amount)
        } else {
            value.saturating_add(amount)
        };
    };
    match action {
        MouseAction::ScrollSidebar => state.select_next(direction * i32::from(amount)),
        MouseAction::ScrollPicker => state.move_picker(direction * i32::from(amount)),
        MouseAction::ScrollHelp => {
            if let Some(help) = state.help.as_mut() {
                update(&mut help.scroll);
            }
        }
        MouseAction::ScrollDetail => update(&mut state.detail_scroll),
        MouseAction::ScrollApproval => update(&mut state.approval_scroll),
        MouseAction::ScrollPorts => update(&mut state.ports_scroll),
        _ => {}
    }
}
