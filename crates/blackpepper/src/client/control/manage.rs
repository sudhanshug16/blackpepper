mod mouse;

use super::super::{actions, ClientMode, ClientState, EmbeddedTerminal};
use super::ClientRuntime;
use crate::keymap::matches_chord;
use termwiz::input::{KeyCode, KeyEvent, Modifiers};

pub(super) use mouse::handle as handle_mouse;

pub(super) fn handle_key(state: &mut ClientState, runtime: &mut ClientRuntime, key: KeyEvent) {
    let modifiers = key.modifiers.remove_positional_mods();
    if handle_command_input(state, runtime, &key, modifiers)
        || handle_scrollable(state, &key, modifiers)
        || cancel_operation(state, runtime, &key, modifiers)
    {
        return;
    }
    if state
        .toggle_chord
        .as_ref()
        .is_some_and(|chord| matches_chord(&key, chord))
    {
        if state.active_workspace.is_some() {
            state.mode = ClientMode::Work;
        }
        return;
    }
    match key.key {
        KeyCode::Char(':') if modifiers == Modifiers::NONE => {
            state.command_active = true;
            state.command_input = ":".to_owned();
        }
        KeyCode::Char('q') if modifiers == Modifiers::NONE => state.should_quit = true,
        KeyCode::UpArrow => state.select_next(-1),
        KeyCode::DownArrow => state.select_next(1),
        KeyCode::Enter => attach_selected(state, runtime),
        KeyCode::Escape if state.active_workspace.is_some() => {
            let workspace_id = state.active_workspace.expect("checked active workspace");
            state.mark_workspace_completions_seen(workspace_id);
            state.mode = ClientMode::Work;
        }
        _ => {}
    }
}

fn handle_command_input(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    key: &KeyEvent,
    modifiers: Modifiers,
) -> bool {
    if !state.command_active {
        return false;
    }
    match key.key {
        KeyCode::Escape => {
            state.command_active = false;
            state.command_input.clear();
        }
        KeyCode::Enter => {
            let input = std::mem::take(&mut state.command_input);
            state.command_active = false;
            state.close_detail();
            match super::super::parse_command(&input) {
                Ok(command) => actions::execute_command(state, runtime, command),
                Err(error) => state.set_output(error),
            }
        }
        KeyCode::Backspace => {
            state.command_input.pop();
        }
        KeyCode::Char(character)
            if modifiers == Modifiers::NONE || modifiers == Modifiers::SHIFT =>
        {
            state.command_input.push(character);
        }
        _ => {}
    }
    true
}

fn handle_scrollable(state: &mut ClientState, key: &KeyEvent, modifiers: Modifiers) -> bool {
    if modifiers != Modifiers::NONE {
        return false;
    }
    let approval = state.pending_approval.is_some();
    let detail = state.detail.is_some();
    if !approval && !detail {
        return false;
    }
    if key.key == KeyCode::Escape {
        if approval {
            state.pending_approval = None;
            state.approval_scroll = 0;
            state.set_output("Approval dismissed; no Worktrunk mutation ran.");
        } else {
            state.close_detail();
        }
        return true;
    }
    let scroll = if approval {
        &mut state.approval_scroll
    } else {
        &mut state.detail_scroll
    };
    match key.key {
        KeyCode::UpArrow => *scroll = scroll.saturating_sub(1),
        KeyCode::DownArrow => *scroll = scroll.saturating_add(1),
        KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
        KeyCode::PageDown => *scroll = scroll.saturating_add(10),
        _ => return false,
    }
    true
}

fn cancel_operation(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    key: &KeyEvent,
    modifiers: Modifiers,
) -> bool {
    if key.key != KeyCode::Escape || modifiers != Modifiers::NONE {
        return false;
    }
    let host_id = state.selected_host.or_else(|| {
        state
            .selected_workspace
            .and_then(|workspace_id| state.host_for_workspace(workspace_id))
    });
    let Some((host_id, label)) = host_id.and_then(|host_id| {
        runtime
            .cancel_host_operation(host_id)
            .map(|label| (host_id, label))
    }) else {
        return false;
    };
    if let Some((_, displayed)) = state.host_operations.get_mut(&host_id) {
        *displayed = format!("Cancelling {label}");
    }
    state.set_output(format!(
        "Cancelling {label}… An uncertain Worktrunk mutation will not be retried."
    ));
    true
}

pub(in crate::client) fn attach_selected(state: &mut ClientState, runtime: &mut ClientRuntime) {
    let Some(workspace_id) = state.selected_workspace else {
        state.set_output("Select or register a workspace first.");
        return;
    };
    if state.terminals.contains_key(&workspace_id) {
        state.active_workspace = Some(workspace_id);
        state.selected_host = state.host_for_workspace(workspace_id);
        state.mark_workspace_completions_seen(workspace_id);
        state.mode = ClientMode::Work;
        return;
    }
    let (rows, cols) = state
        .terminal_area
        .map(|area| (area.height, area.width))
        .unwrap_or((24, 80));
    let Some(host_id) = state.host_for_workspace(workspace_id) else {
        state.set_output("The selected workspace host is unavailable.");
        return;
    };
    let label = "Preparing and attaching workspace session".to_owned();
    let result = runtime.start_host_operation(
        host_id,
        label.clone(),
        crate::client::runtime::HostOperationContext::Attach { workspace_id },
        state.event_tx.clone(),
        Box::new(move |runtime| {
            runtime.attach_workspace(workspace_id, rows, cols).map(
                |(process, provisional_clients)| {
                    crate::client::runtime::HostOperationValue::Attached {
                        workspace_id,
                        process,
                        provisional_clients,
                    }
                },
            )
        }),
    );
    match result {
        Ok(token) => {
            state
                .host_operations
                .insert(host_id, (token, label.clone()));
            state.set_output(format!("{label}… Press Esc in Manage mode to cancel."));
        }
        Err(error) => state.set_output(error),
    }
}

pub(in crate::client) fn apply_attachment(
    state: &mut ClientState,
    workspace_id: crate::core::WorkspaceId,
    process: crate::transport::PtyProcess,
    provisional_clients: usize,
) -> Result<(), String> {
    let (rows, cols) = state
        .terminal_area
        .map(|area| (area.height, area.width))
        .unwrap_or((24, 80));
    let terminal = EmbeddedTerminal::new(
        workspace_id,
        process,
        rows,
        cols,
        state.config.ui.foreground,
        state.config.ui.background,
        state.event_tx.clone(),
    )
    .map_err(|error| error.to_string())?;
    state.terminals.insert(workspace_id, terminal);
    state.connected_clients.remove(&workspace_id);
    state.active_workspace = Some(workspace_id);
    state.selected_workspace = Some(workspace_id);
    state.selected_host = state.host_for_workspace(workspace_id);
    state.mark_workspace_completions_seen(workspace_id);
    state.mode = ClientMode::Work;
    if provisional_clients > 1 {
        state.set_output(format!(
            "{provisional_clients} Zellij clients attached — input, scrolling, search, and selection are shared."
        ));
    } else {
        state.clear_output();
    }
    Ok(())
}

#[cfg(test)]
mod tests;
