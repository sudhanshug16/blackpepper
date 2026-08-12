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
            state.command_selection = None;
        }
        // Enter runs the highlighted candidate when there is one, and what was
        // typed when there is not. Completing onto a command that still wants
        // an argument leaves the bar open rather than running something the
        // parser would reject.
        KeyCode::Enter => {
            if state.command_selection.is_some() {
                let completed = apply_completion(state);
                if !completed {
                    return true;
                }
            }
            let input = std::mem::take(&mut state.command_input);
            state.command_active = false;
            state.command_selection = None;
            state.close_detail();
            match crate::client::parse_command(&input) {
                Ok(command) => actions::execute_command(state, runtime, command),
                Err(error) => state.set_output(error),
            }
        }
        // Tab takes the highlighted candidate, or the first one when nothing
        // is highlighted yet.
        KeyCode::Tab => {
            apply_completion(state);
        }
        KeyCode::UpArrow => {
            // Stepping off the top of the list returns to what was typed,
            // which is the only way back to it without retyping.
            state.command_selection = match state.command_selection {
                Some(0) | None => None,
                Some(index) => Some(index - 1),
            };
        }
        KeyCode::DownArrow => {
            let count = completion_count(state);
            if count > 0 {
                state.command_selection = Some(match state.command_selection {
                    None => 0,
                    Some(index) => index.saturating_add(1).min(count - 1),
                });
            }
        }
        KeyCode::Backspace => {
            state.command_input.pop();
            state.command_selection = None;
        }
        KeyCode::Char(character)
            if modifiers == Modifiers::NONE || modifiers == Modifiers::SHIFT =>
        {
            state.command_input.push(character);
            state.command_selection = None;
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

/// Replace the typed text with the highlighted candidate. Returns whether the
/// result is a complete command; `false` means a placeholder was dropped and
/// the bar is now waiting for an argument.
fn apply_completion(state: &mut ClientState) -> bool {
    let body = state
        .command_input
        .strip_prefix(':')
        .unwrap_or(&state.command_input)
        .to_owned();
    let candidates = crate::client::completion::candidates(state, &body);
    let Some(candidate) = candidates.get(state.command_selection.unwrap_or(0)) else {
        return true;
    };
    // Placeholder syntax is not runnable text. A required `<arg>` leaves a
    // trailing space and waits; an optional `[arg]` is dropped and the command
    // runs without it, because that is what "optional" means.
    let mut words = Vec::new();
    let mut wants_argument = false;
    for word in candidate.value.split_whitespace() {
        if word.starts_with('<') {
            wants_argument = true;
            break;
        }
        if word.starts_with('[') {
            break;
        }
        words.push(word);
    }
    let value = words.join(" ");
    let complete = !wants_argument;
    let trailing = if complete { "" } else { " " };
    state.command_input = format!(":{value}{trailing}");
    state.command_selection = None;
    complete
}
