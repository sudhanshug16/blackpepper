use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::commands::CommandPhase;
use crate::events::AppEvent;
use crate::keymap::matches_chord;
use crate::terminal::key_event_to_bytes;

use super::command::handle_command_input;
use super::mouse::handle_mouse_event;
use super::overlay::{
    handle_overlay_key, handle_prompt_overlay_key, handle_tab_overlay_key, open_tab_overlay,
    open_workspace_overlay,
};
use super::search::{handle_search_input, search_next, search_prev};
use crate::app::state::{App, Mode};
use super::terminal::{
    clear_selection, copy_selection, handle_scrollback_key, process_terminal_output,
};
use super::workspace::{
    active_terminal_mut, close_tab_by_id, ensure_manage_mode_without_workspace, enter_work_mode,
    request_refresh, set_active_workspace,
};

/// Main event dispatcher.
pub fn handle_event(app: &mut App, event: AppEvent) {
    match event {
        AppEvent::Input(key) => handle_key(app, key),
        AppEvent::PtyOutput(id, bytes) => {
            process_terminal_output(app, id, &bytes);
        }
        AppEvent::PtyExit(id) => {
            close_tab_by_id(app, id);
        }
        AppEvent::Mouse(mouse) => {
            handle_mouse_event(app, mouse);
        }
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
                let stream_output = app.command_overlay.output.trim().to_string();
                app.command_overlay.visible = false;
                app.command_overlay.title.clear();
                if !stream_output.is_empty() {
                    app.set_output(stream_output);
                }
            }
        }
        AppEvent::CommandDone { name, args, result } => {
            let stream_output = app.command_overlay.output.trim().to_string();
            let message = if stream_output.is_empty() {
                result.message
            } else if result.message.trim().is_empty() || result.message.trim() == stream_output {
                stream_output
            } else {
                format!("{stream_output}\n\n{}", result.message)
            };
            app.loading = None;
            app.command_overlay.visible = false;
            app.command_overlay.output.clear();
            app.command_overlay.title.clear();
            app.set_output(message);
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
                            if app.active_workspace.as_deref() == Some(name.as_str()) {
                                app.active_workspace = None;
                                app.tabs.remove(name);
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
    }
}

fn handle_key(app: &mut App, key: KeyEvent) {
    // Ignore key releases; we only want press/repeat events to reach the PTY.
    if key.kind == KeyEventKind::Release {
        return;
    }
    if app.loading.is_some() {
        return;
    }
    if app.overlay.visible {
        handle_overlay_key(app, key);
        return;
    }
    if app.tab_overlay.visible {
        handle_tab_overlay_key(app, key);
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
    if app.search.active {
        handle_search_input(app, key);
        return;
    }

    // Escape clears selection in work mode
    if app.mode == Mode::Work
        && app.selection.active
        && key.code == KeyCode::Esc
        && key.modifiers.is_empty()
    {
        clear_selection(app);
        return;
    }

    if app.mode == Mode::Work && handle_scrollback_key(app, key) {
        return;
    }

    if handle_tab_shortcut(app, key) {
        return;
    }

    // Toggle mode chord
    if let Some(chord) = &app.toggle_chord {
        if matches_chord(key, chord) {
            if app.mode == Mode::Work {
                app.mode = Mode::Manage;
            } else {
                enter_work_mode(app);
            }
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

    // Switch tab chord
    if let Some(chord) = &app.switch_tab_chord {
        if matches_chord(key, chord) {
            open_tab_overlay(app);
            return;
        }
    }

    // Refresh UI chord (manage mode only)
    if app.mode == Mode::Manage {
        if let Some(chord) = &app.refresh_chord {
            if matches_chord(key, chord) {
                request_refresh(app, None);
                return;
            }
        }
    }

    // Ctrl+Shift+F for search
    if app.mode == Mode::Work
        && key.code == KeyCode::Char('f')
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && key.modifiers.contains(KeyModifiers::SHIFT)
    {
        app.search.active = true;
        return;
    }

    // Ctrl+Shift+C for copy
    if app.mode == Mode::Work
        && key.code == KeyCode::Char('c')
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && key.modifiers.contains(KeyModifiers::SHIFT)
        && copy_selection(app)
    {
        return;
    }

    // Ctrl+Shift+N for next search match
    if app.mode == Mode::Work
        && key.code == KeyCode::Char('n')
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && key.modifiers.contains(KeyModifiers::SHIFT)
    {
        search_next(app);
        return;
    }

    // Ctrl+Shift+P for previous search match
    if app.mode == Mode::Work
        && key.code == KeyCode::Char('p')
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && key.modifiers.contains(KeyModifiers::SHIFT)
    {
        search_prev(app);
        return;
    }

    // Manage mode: open command bar with ':'
    if app.mode == Mode::Manage && key.code == KeyCode::Char(':') {
        super::command::open_command(app);
        return;
    }

    // Manage mode: quit with 'q'
    if app.mode == Mode::Manage && key.code == KeyCode::Char('q') && key.modifiers.is_empty() {
        app.should_quit = true;
        return;
    }

    // Manage mode: escape returns to work mode
    if app.mode == Mode::Manage && key.code == KeyCode::Esc {
        enter_work_mode(app);
        return;
    }

    // Work mode: send keys to terminal
    if app.mode == Mode::Work {
        if app.selection.active {
            clear_selection(app);
        }
        if let Some(terminal) = active_terminal_mut(app) {
            if terminal.scrollback() > 0 {
                terminal.scroll_to_bottom();
            }
            if let Some(bytes) = key_event_to_bytes(key) {
                terminal.write_bytes(&bytes);
            }
        }
    }
}

fn handle_tab_shortcut(app: &mut App, key: KeyEvent) -> bool {
    if app.mode != Mode::Work {
        return false;
    }

    // Ctrl+Tab / Ctrl+Shift+Tab for tab switching
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Tab {
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            super::workspace::tab_prev(app);
        } else {
            super::workspace::tab_next(app);
        }
        return true;
    }

    // Alt+1-9 for direct tab selection
    if key.modifiers.contains(KeyModifiers::ALT) {
        if let KeyCode::Char(ch) = key.code {
            if ('1'..='9').contains(&ch) {
                let index = ch.to_digit(10).unwrap_or(1) as usize;
                super::workspace::tab_select(app, index.saturating_sub(1));
                return true;
            }
        }
    }

    false
}
