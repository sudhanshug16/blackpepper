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
        KeyCode::Escape => {
            state.command_active = false;
            state.command_input.clear();
            state.command_selection = 0;
        }
        KeyCode::Enter => {
            let input = std::mem::take(&mut state.command_input);
            state.command_active = false;
            state.command_selection = 0;
            state.close_detail();
            match crate::client::parse_command(&input) {
                Ok(command) => actions::execute_command(state, runtime, command),
                Err(error) => state.set_output(error),
            }
        }
        // Tab takes the highlighted candidate; the arrows move between them.
        // Completion is always optional — typing the command out in full
        // behaves exactly as it did before.
        KeyCode::Tab => apply_completion(state),
        KeyCode::UpArrow => state.command_selection = state.command_selection.saturating_sub(1),
        KeyCode::DownArrow => {
            let count = completion_count(state);
            state.command_selection = state
                .command_selection
                .saturating_add(1)
                .min(count.saturating_sub(1));
        }
        KeyCode::Backspace => {
            state.command_input.pop();
            state.command_selection = 0;
        }
        KeyCode::Char(character)
            if modifiers == Modifiers::NONE || modifiers == Modifiers::SHIFT =>
        {
            state.command_input.push(character);
            state.command_selection = 0;
        }
        _ => {}
    }
    true
}

fn completion_count(state: &ClientState) -> usize {
    let body = state
        .command_input
        .strip_prefix(':')
        .unwrap_or(&state.command_input)
        .to_owned();
    crate::client::completion::candidates(state, &body).len()
}

fn apply_completion(state: &mut ClientState) {
    let body = state
        .command_input
        .strip_prefix(':')
        .unwrap_or(&state.command_input)
        .to_owned();
    let candidates = crate::client::completion::candidates(state, &body);
    let Some(candidate) = candidates.get(state.command_selection) else {
        return;
    };
    // Placeholder syntax is not runnable text, so completing onto a bare
    // command leaves a trailing space and waits for the argument.
    let value = candidate
        .value
        .split_whitespace()
        .take_while(|word| !word.starts_with('<') && !word.starts_with('['))
        .collect::<Vec<_>>()
        .join(" ");
    let trailing = if value == candidate.value { "" } else { " " };
    state.command_input = format!(":{value}{trailing}");
    state.command_selection = 0;
}
