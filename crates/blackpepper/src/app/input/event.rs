use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::commands::CommandPhase;
use crate::events::AppEvent;
use crate::keymap::matches_chord;

use super::command::handle_command_input;
use super::overlay::{handle_overlay_key, handle_prompt_overlay_key, open_workspace_overlay};
use super::terminal::process_terminal_output;
use super::workspace::{
    active_terminal_mut, close_session_by_id, ensure_manage_mode_without_workspace, enter_work_mode,
    request_refresh, set_active_workspace,
};
use crate::app::state::{App, Mode};

/// Main event dispatcher.
pub fn handle_event(app: &mut App, event: AppEvent) {
    match event {
        AppEvent::Input(key) => handle_key(app, key),
        AppEvent::RawInput(bytes) => handle_raw_input(app, bytes),
        AppEvent::PtyOutput(id, bytes) => {
            process_terminal_output(app, id, &bytes);
        }
        AppEvent::PtyExit(id) => close_session_by_id(app, id),
        AppEvent::Resize(_, _) => {}
        AppEvent::CommandOutput { name, chunk } => {
            if !app.command_overlay.visible {
                app.command_overlay.visible = true;
                app.command_overlay.title = if let Some(label) = &app.loading {
                    label.clone()
                } else {
                    name
                };
            }
            app.command_overlay.output.push_str(&chunk);
        }
        AppEvent::CommandPhaseComplete { phase } => {
            if matches!(phase, CommandPhase::Agent) {
                // Keep the overlay open; command completion will finalize output.
            }
        }
        AppEvent::CommandDone { name, args, result } => {
            let stream_output = app.command_overlay.output.trim().to_string();
            let result_message = result.message.clone();
            let message = if stream_output.is_empty() {
                result_message.clone()
            } else if result_message.trim().is_empty()
                || result_message.trim() == stream_output
                || stream_output.contains(result_message.trim())
            {
                stream_output.clone()
            } else {
                format!("{stream_output}\n\n{result_message}")
            };
            app.loading = None;
            let overlay_visible = app.command_overlay.visible && !stream_output.is_empty();
            if overlay_visible {
                if app.command_overlay.title.is_empty() {
                    app.command_overlay.title = "Command output (Esc to close)".to_string();
                } else if !app.command_overlay.title.contains("Esc") {
                    app.command_overlay.title =
                        format!("{} (Esc to close)", app.command_overlay.title);
                }
                if !result.ok
                    && !result_message.trim().is_empty()
                    && !app.command_overlay.output.contains(result_message.trim())
                {
                    if !app.command_overlay.output.ends_with('\n') {
                        app.command_overlay.output.push('\n');
                    }
                    app.command_overlay.output.push_str(result_message.trim());
                    app.command_overlay.output.push('\n');
                }
            }
            if !overlay_visible {
                app.set_output(message);
            }
            if name == "workspace" {
                if let Some(subcommand) = args.first() {
                    if subcommand == "create" && result.ok {
                        if let Some(name) = result.data.as_deref() {
                            if set_active_workspace(app, name).is_ok() {
                                app.set_output(format!("Active workspace: {name}"));
                            }
                        }
                        enter_work_mode(app);
                    }
                    if subcommand == "destroy" && result.ok {
                        if let Some(name) = args.get(1) {
                            app.sessions.remove(name);
                            if app.active_workspace.as_deref() == Some(name.as_str()) {
                                app.active_workspace = None;
                                if let Some(root) = &app.repo_root {
                                    app.cwd = root.clone();
                                }
                                ensure_manage_mode_without_workspace(app);
                            }
                        }
                    }
                }
            }
        }
        AppEvent::RepoStatusUpdated { cwd, status } => {
            if cwd == app.cwd {
                app.repo_status = status;
            }
        }
    }
}

fn handle_key(app: &mut App, key: KeyEvent) {
    // Ignore key releases; manage mode only needs press/repeat events.
    if key.kind == KeyEventKind::Release {
        return;
    }
    if app.mode == Mode::Work {
        return;
    }

    if app.loading.is_some() {
        return;
    }
    if app.command_overlay.visible {
        if key.code == KeyCode::Esc && key.modifiers.is_empty() {
            app.command_overlay.visible = false;
            app.command_overlay.output.clear();
            app.command_overlay.title.clear();
        }
        return;
    }
    if app.overlay.visible {
        handle_overlay_key(app, key);
        return;
    }
    if app.prompt_overlay.visible {
        handle_prompt_overlay_key(app, key);
        return;
    }
    if app.command_active {
        handle_command_input(app, key);
        return;
    }

    // Toggle mode chord
    if let Some(chord) = &app.toggle_chord {
        if matches_chord(key, chord) {
            enter_work_mode(app);
            return;
        }
    }

    // Switch workspace chord
    if let Some(chord) = &app.switch_chord {
        if matches_chord(key, chord) {
            open_workspace_overlay(app);
            return;
        }
    }

    // Refresh UI chord (manage mode only)
    if let Some(chord) = &app.refresh_chord {
        if matches_chord(key, chord) {
            request_refresh(app, None);
            return;
        }
    }

    // Manage mode: open command bar with ':'
    if key.code == KeyCode::Char(':') {
        super::command::open_command(app);
        return;
    }

    // Manage mode: quit with 'q'
    if key.code == KeyCode::Char('q') && key.modifiers.is_empty() {
        app.should_quit = true;
        return;
    }

    // Manage mode: escape returns to work mode
    if key.code == KeyCode::Esc {
        enter_work_mode(app);
        return;
    }
}

fn handle_raw_input(app: &mut App, bytes: Vec<u8>) {
    if app.mode != Mode::Work || bytes.is_empty() {
        return;
    }
    let toggle = app.work_toggle_byte;
    if let Some(pos) = bytes.iter().position(|byte| *byte == toggle) {
        if pos > 0 {
            if let Some(terminal) = active_terminal_mut(app) {
                terminal.write_bytes(&bytes[..pos]);
            }
        }
        app.set_mode(Mode::Manage);
        return;
    }
    if let Some(terminal) = active_terminal_mut(app) {
        terminal.write_bytes(&bytes);
    }
}
