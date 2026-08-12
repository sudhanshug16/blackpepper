//! Key handling for the surfaces that capture input while they are open.
//!
//! The picker, grouped help, and the command bar each swallow every key they
//! see. Keeping them here means `handle_key` reads as a short list of who gets
//! first refusal, rather than as one long match.

use super::attach_selected;
use crate::client::runtime::ClientRuntime;
use crate::client::{actions, ClientState};
use termwiz::input::{KeyCode, KeyEvent, Modifiers};

/// The workspace picker: type to filter across every host, enter to attach.
pub(super) fn handle_picker(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    key: &KeyEvent,
    modifiers: Modifiers,
) -> bool {
    if state.picker.is_none() {
        return false;
    }
    match key.key {
        KeyCode::Escape => state.picker = None,
        KeyCode::Enter => {
            let choice = state.picker_choice();
            state.picker = None;
            if let Some(workspace_id) = choice {
                state.selected_workspace = Some(workspace_id);
                state.selected_host = state.host_for_workspace(workspace_id);
                attach_selected(state, runtime);
            }
        }
        KeyCode::UpArrow => state.move_picker(-1),
        KeyCode::DownArrow => state.move_picker(1),
        KeyCode::Backspace => {
            if let Some(picker) = state.picker.as_mut() {
                picker.filter.pop();
                picker.selected = 0;
            }
        }
        KeyCode::Char(character)
            if modifiers == Modifiers::NONE || modifiers == Modifiers::SHIFT =>
        {
            if let Some(picker) = state.picker.as_mut() {
                picker.filter.push(character);
                // A narrower list invalidates the old index outright.
                picker.selected = 0;
            }
        }
        _ => {}
    }
    true
}

pub(super) fn handle_help(state: &mut ClientState, key: &KeyEvent, modifiers: Modifiers) -> bool {
    let Some(help) = state.help else {
        return false;
    };
    if modifiers != Modifiers::NONE {
        return false;
    }
    let mut scroll = help.scroll;
    match key.key {
        KeyCode::Escape | KeyCode::Char('q') => {
            state.help = None;
            return true;
        }
        KeyCode::UpArrow => scroll = scroll.saturating_sub(1),
        KeyCode::DownArrow => scroll = scroll.saturating_add(1),
        KeyCode::PageUp => scroll = scroll.saturating_sub(10),
        KeyCode::PageDown => scroll = scroll.saturating_add(10),
        _ => return true,
    }
    state.help = Some(crate::client::state::HelpView { scroll });
    true
}

pub(super) fn handle_command_input(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    key: &KeyEvent,
    modifiers: Modifiers,
) -> bool {
    if !state.command_active {
        return false;
    }
    match key.key {
        KeyCode::Escape => close_command(state),
        KeyCode::Enter => {
            if state.command_selection.is_some() && !apply_completion(state) {
                return true;
            }
            let input = state.command_input.clone();
            match crate::client::parse_command(&input) {
                Ok(command) => {
                    close_command(state);
                    state.close_detail();
                    actions::execute_command(state, runtime, command);
                }
                Err(error) => {
                    state.command_error = Some(error);
                }
            }
        }
        // Tab takes the highlighted candidate; the arrows move between them.
        // Completion is always optional — typing the command out in full
        // behaves exactly as it did before.
        KeyCode::Tab if modifiers.contains(Modifiers::SHIFT) => move_completion(state, -1),
        KeyCode::Tab => {
            apply_completion(state);
        }
        KeyCode::UpArrow => move_completion(state, -1),
        KeyCode::DownArrow => move_completion(state, 1),
        KeyCode::Backspace => {
            if state.command_input == ":" {
                close_command(state);
            } else {
                state.command_input.pop();
                reset_command_feedback(state);
            }
        }
        KeyCode::Char(character)
            if modifiers == Modifiers::NONE || modifiers == Modifiers::SHIFT =>
        {
            state.command_input.push(character);
            reset_command_feedback(state);
        }
        _ => {}
    }
    true
}

pub(super) fn open_command(state: &mut ClientState) {
    state.command_active = true;
    state.command_input = ":".to_owned();
    reset_command_feedback(state);
}

pub(super) fn prefill_command(state: &mut ClientState, input: impl Into<String>) {
    state.command_active = true;
    state.command_input = input.into();
    if !state.command_input.starts_with(':') {
        state.command_input.insert(0, ':');
    }
    reset_command_feedback(state);
}

pub(super) fn close_command(state: &mut ClientState) {
    state.command_active = false;
    state.command_input.clear();
    reset_command_feedback(state);
}

pub(super) fn completion_count(state: &ClientState) -> usize {
    let body = state
        .command_input
        .strip_prefix(':')
        .unwrap_or(&state.command_input)
        .to_owned();
    crate::client::completion::candidates(state, &body).len()
}

pub(super) fn choose_completion(state: &mut ClientState, index: usize) {
    let count = completion_count(state);
    state.command_selection = (count > 0).then_some(index.min(count - 1));
    apply_completion(state);
}

/// Apply the highlighted candidate, or the first candidate for Tab. Returns
/// whether the result is a complete command that Enter may execute.
pub(super) fn apply_completion(state: &mut ClientState) -> bool {
    let body = state
        .command_input
        .strip_prefix(':')
        .unwrap_or(&state.command_input)
        .to_owned();
    let candidates = crate::client::completion::candidates(state, &body);
    let Some(candidate) = candidates.get(state.command_selection.unwrap_or(0)) else {
        return true;
    };
    state.command_input = format!(
        ":{}{}",
        candidate.value,
        if candidate.expects_more { " " } else { "" }
    );
    let complete = !candidate.expects_more;
    reset_command_feedback(state);
    complete
}

fn move_completion(state: &mut ClientState, direction: i32) {
    let count = completion_count(state);
    if count == 0 {
        state.command_selection = None;
        return;
    }
    state.command_selection = if direction < 0 {
        match state.command_selection {
            None | Some(0) => None,
            Some(index) => Some(index - 1),
        }
    } else {
        Some(match state.command_selection {
            None => 0,
            Some(index) => index.saturating_add(1).min(count - 1),
        })
    };
    state.command_error = None;
}

fn reset_command_feedback(state: &mut ClientState) {
    state.command_selection = None;
    state.command_error = None;
}
