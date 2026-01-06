//! Input event handling.
//!
//! Handles keyboard and mouse events, routing them to appropriate
//! handlers based on current mode, overlays, and focus.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use arboard::Clipboard;
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

use crate::commands::pr;
use crate::commands::{
    complete_command_input, parse_command, run_command, CommandContext, CommandSource,
};
use crate::config::{load_config, save_user_agent_provider};
use crate::events::AppEvent;
use crate::keymap::matches_chord;
use crate::state::{record_active_workspace, remove_active_workspace};
use crate::terminal::{key_event_to_bytes, mouse_event_to_bytes, TerminalSession};
use crate::workspaces::{list_workspace_names, workspace_absolute_path};

use super::state::{
    App, CellPos, Mode, PendingCommand, SearchMatch, WorkspaceTab, WorkspaceTabs,
    MAX_TAB_LABEL_LEN, SCROLL_LINES,
};

const NO_ACTIVE_WORKSPACE_HINT: &str =
    "No active workspace yet. Create one with :workspace create <name>.";
const REFRESH_USAGE: &str = "Usage: :refresh";

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
        AppEvent::CommandDone { name, args, result } => {
            app.loading = None;
            app.set_output(result.message);
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
        open_command(app);
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
            tab_prev(app);
        } else {
            tab_next(app);
        }
        return true;
    }

    // Alt+1-9 for direct tab selection
    if key.modifiers.contains(KeyModifiers::ALT) {
        if let KeyCode::Char(ch) = key.code {
            if ('1'..='9').contains(&ch) {
                let index = ch.to_digit(10).unwrap_or(1) as usize;
                tab_select(app, index.saturating_sub(1));
                return true;
            }
        }
    }

    false
}

fn handle_search_input(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.search.active = false;
            app.search.query.clear();
            app.search.matches.clear();
            app.search.active_index = 0;
        }
        KeyCode::Enter => {
            app.search.active = false;
            if app.search.query.trim().is_empty() {
                app.search.matches.clear();
                app.search.active_index = 0;
            }
        }
        KeyCode::Backspace => {
            app.search.query.pop();
        }
        KeyCode::Char(ch) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                app.search.query.push(ch);
            }
        }
        _ => {}
    }
}

fn search_next(app: &mut App) {
    if app.search.matches.is_empty() {
        return;
    }
    app.search.active_index = (app.search.active_index + 1) % app.search.matches.len();
}

fn search_prev(app: &mut App) {
    if app.search.matches.is_empty() {
        return;
    }
    if app.search.active_index == 0 {
        app.search.active_index = app.search.matches.len() - 1;
    } else {
        app.search.active_index -= 1;
    }
}

fn handle_scrollback_key(app: &mut App, key: KeyEvent) -> bool {
    if !key.modifiers.contains(KeyModifiers::SHIFT) {
        return false;
    }
    let Some(terminal) = active_terminal_mut(app) else {
        return false;
    };
    if terminal.alternate_screen() {
        return false;
    }
    let page = terminal.rows().saturating_sub(1) as isize;
    match key.code {
        KeyCode::PageUp => {
            terminal.scroll_lines(page);
            true
        }
        KeyCode::PageDown => {
            terminal.scroll_lines(-page);
            true
        }
        KeyCode::Home => {
            terminal.scroll_to_top();
            true
        }
        KeyCode::End => {
            terminal.scroll_to_bottom();
            true
        }
        _ => false,
    }
}

fn handle_overlay_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.overlay.visible = false;
        }
        KeyCode::Enter => {
            if let Some(name) = app.overlay.items.get(app.overlay.selected) {
                let name = name.clone();
                match set_active_workspace(app, &name) {
                    Ok(()) => app.set_output(format!("Active workspace: {name}")),
                    Err(err) => app.set_output(err),
                }
            }
            app.overlay.visible = false;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            move_overlay_selection(app, -1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            move_overlay_selection(app, 1);
        }
        _ => {}
    }
}

fn move_overlay_selection(app: &mut App, delta: isize) {
    if app.overlay.items.is_empty() {
        return;
    }
    let len = app.overlay.items.len() as isize;
    let mut next = app.overlay.selected as isize + delta;
    if next < 0 {
        next = len - 1;
    } else if next >= len {
        next = 0;
    }
    app.overlay.selected = next as usize;
}

fn handle_tab_overlay_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.tab_overlay.visible = false;
        }
        KeyCode::Enter => {
            if let Some(index) = app.tab_overlay.items.get(app.tab_overlay.selected) {
                tab_select(app, *index);
            }
            app.tab_overlay.visible = false;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            move_tab_overlay_selection(app, -1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            move_tab_overlay_selection(app, 1);
        }
        _ => {}
    }
}

fn handle_prompt_overlay_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.prompt_overlay.visible = false;
            app.pending_command = None;
        }
        KeyCode::Enter => {
            let selected = app
                .prompt_overlay
                .items
                .get(app.prompt_overlay.selected)
                .cloned();
            app.prompt_overlay.visible = false;
            if let Some(provider) = selected {
                if let Err(err) = save_user_agent_provider(&provider) {
                    app.set_output(format!("Failed to save agent provider: {err}"));
                    app.pending_command = None;
                    return;
                }
                app.config.agent.provider = Some(provider.clone());
                if let Some(pending) = app.pending_command.take() {
                    start_command(app, &pending.name, pending.args);
                } else {
                    app.set_output(format!("Saved agent provider: {provider}"));
                }
            } else {
                app.pending_command = None;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            move_prompt_overlay_selection(app, -1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            move_prompt_overlay_selection(app, 1);
        }
        _ => {}
    }
}

fn move_tab_overlay_selection(app: &mut App, delta: isize) {
    if app.tab_overlay.items.is_empty() {
        return;
    }
    let len = app.tab_overlay.items.len() as isize;
    let mut next = app.tab_overlay.selected as isize + delta;
    if next < 0 {
        next = len - 1;
    } else if next >= len {
        next = 0;
    }
    app.tab_overlay.selected = next as usize;
}

fn move_prompt_overlay_selection(app: &mut App, delta: isize) {
    if app.prompt_overlay.items.is_empty() {
        return;
    }
    let len = app.prompt_overlay.items.len() as isize;
    let mut next = app.prompt_overlay.selected as isize + delta;
    if next < 0 {
        next = len - 1;
    } else if next >= len {
        next = 0;
    }
    app.prompt_overlay.selected = next as usize;
}

fn handle_command_input(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.command_active = false;
            app.command_input.clear();
        }
        KeyCode::Enter => {
            let input = app.command_input.clone();
            app.command_active = false;
            app.command_input.clear();
            execute_command(app, &input);
        }
        KeyCode::Tab => {
            if let Some(completed) = complete_command_input(&app.command_input) {
                app.command_input = completed;
            }
        }
        KeyCode::Backspace => {
            app.command_input.pop();
            if app.command_input.is_empty() {
                app.command_active = false;
            }
        }
        KeyCode::Char(ch) => {
            app.command_input.push(ch);
        }
        _ => {}
    }
}

fn open_command(app: &mut App) {
    app.command_active = true;
    app.command_input = ":".to_string();
}

fn execute_command(app: &mut App, raw: &str) {
    let parsed = match parse_command(raw) {
        Ok(parsed) => parsed,
        Err(err) => {
            app.set_output(format!("Error: {}", err.error));
            return;
        }
    };

    if parsed.name == "quit" || parsed.name == "q" {
        app.should_quit = true;
        return;
    }

    if parsed.name == "workspace" {
        handle_workspace_command(app, &parsed.args);
        return;
    }

    if parsed.name == "tab" {
        handle_tab_command(app, &parsed.args);
        return;
    }

    if parsed.name == "pr" {
        handle_pr_command(app, &parsed.name, &parsed.args);
        return;
    }

    if parsed.name == "debug" {
        handle_debug_command(app, &parsed.args);
        return;
    }

    if parsed.name == "export" {
        handle_export_command(app, &parsed.args);
        return;
    }

    if parsed.name == "refresh" {
        handle_refresh_command(app, &parsed.args);
        return;
    }

    let result = run_command(
        &parsed.name,
        &parsed.args,
        &CommandContext {
            cwd: app.cwd.clone(),
            repo_root: app.repo_root.clone(),
            workspace_root: app.config.workspace.root.clone(),
            source: CommandSource::Tui,
        },
    );
    app.set_output(result.message);
}

fn handle_workspace_command(app: &mut App, args: &[String]) {
    let Some(subcommand) = args.first() else {
        app.set_output("Usage: :workspace <list|switch|create|destroy>".to_string());
        return;
    };
    match subcommand.as_str() {
        "list" => {
            open_workspace_overlay(app);
        }
        "switch" => {
            if let Some(root) = app.repo_root.as_ref() {
                let names = list_workspace_names(root, &app.config.workspace.root);
                prune_missing_active_workspace(app, &names);
            }
            let Some(name) = args.get(1) else {
                app.set_output("Usage: :workspace switch <name>".to_string());
                return;
            };
            match set_active_workspace(app, name) {
                Ok(()) => app.set_output(format!("Active workspace: {name}")),
                Err(err) => app.set_output(err),
            }
        }
        "create" | "destroy" => {
            if subcommand == "destroy" && args.len() == 1 {
                if let Some(active) = app.active_workspace.as_ref() {
                    let mut args = args.to_vec();
                    args.push(active.clone());
                    start_command(app, "workspace", args);
                    return;
                }
            }
            start_command(app, "workspace", args.to_vec());
        }
        _ => {
            app.set_output("Usage: :workspace <list|switch|create|destroy>".to_string());
        }
    }
}

fn handle_tab_command(app: &mut App, args: &[String]) {
    let Some(subcommand) = args.first() else {
        app.set_output("Usage: :tab <new|rename|close|next|prev|switch>".to_string());
        return;
    };

    match subcommand.as_str() {
        "new" => {
            let name = args.get(1).cloned();
            match create_tab_for_active(app, 24, 80, name) {
                Ok(name) => {
                    app.set_output(format!("Opened tab: {name}"));
                    enter_work_mode(app);
                }
                Err(err) => app.set_output(err),
            }
        }
        "rename" => {
            let Some(name) = args.get(1) else {
                app.set_output("Usage: :tab rename <name>".to_string());
                return;
            };
            match rename_active_tab(app, name) {
                Ok(()) => app.set_output(format!("Renamed tab to {name}")),
                Err(err) => app.set_output(err),
            }
        }
        "close" => match close_active_tab(app) {
            Ok(message) => app.set_output(message),
            Err(err) => app.set_output(err),
        },
        "next" => tab_next(app),
        "prev" => tab_prev(app),
        "switch" => {
            if let Some(arg) = args.get(1) {
                tab_select_by_arg(app, arg);
            } else {
                open_tab_overlay(app);
            }
        }
        _ => {
            if args.len() == 1 {
                tab_select_by_arg(app, subcommand);
            } else {
                app.set_output("Usage: :tab <new|rename|close|next|prev|switch>".to_string());
            }
        }
    }
}

fn handle_pr_command(app: &mut App, name: &str, args: &[String]) {
    if app.active_workspace.is_none() {
        app.set_output(NO_ACTIVE_WORKSPACE_HINT.to_string());
        return;
    }
    if needs_pr_provider_selection(app, args) {
        open_pr_provider_overlay(
            app,
            PendingCommand {
                name: name.to_string(),
                args: args.to_vec(),
            },
        );
        return;
    }
    start_command(app, name, args.to_vec());
}

fn needs_pr_provider_selection(app: &App, args: &[String]) -> bool {
    let Some(subcommand) = args.first() else {
        return false;
    };
    if subcommand != "create" {
        return false;
    }
    app.config.agent.provider.is_none() && app.config.agent.command.is_none()
}

fn handle_debug_command(app: &mut App, args: &[String]) {
    let Some(subcommand) = args.first() else {
        app.set_output("Usage: :debug <mouse>".to_string());
        return;
    };
    match subcommand.as_str() {
        "mouse" => {
            app.mouse_debug = !app.mouse_debug;
            let state = if app.mouse_debug { "on" } else { "off" };
            if app.mouse_debug {
                let path = app
                    .mouse_log_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "<unavailable>".to_string());
                app.set_output(format!("Mouse debug {state}. Logging to {path}."));
            } else {
                app.set_output(format!("Mouse debug {state}."));
            }
        }
        _ => {
            app.set_output("Usage: :debug <mouse>".to_string());
        }
    }
}

fn handle_export_command(app: &mut App, args: &[String]) {
    if !args.is_empty() {
        app.set_output("Usage: :export".to_string());
        return;
    }
    let editor = match find_editor_binary() {
        Some(editor) => editor,
        None => {
            app.set_output("vim/vi not found. Install vim to use :export.".to_string());
            return;
        }
    };
    let (rows, cols) = app
        .terminal_area
        .map(|area| (area.height.max(1), area.width.max(1)))
        .unwrap_or((24, 80));

    let contents = match active_tab_mut(app) {
        Some(tab) => tab.terminal.scrollback_contents(),
        None => {
            app.set_output("No active workspace.".to_string());
            return;
        }
    };

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let file_path = std::env::temp_dir().join(format!("blackpepper-scrollback-{timestamp}.txt"));
    if let Err(err) = fs::write(&file_path, contents) {
        app.set_output(format!("Failed to write export file: {err}"));
        return;
    }

    let tab_name = match spawn_tab(app, rows, cols, None) {
        Ok(name) => name,
        Err(err) => {
            app.set_output(err);
            return;
        }
    };

    if let Some(tab) = active_tab_mut(app) {
        let quoted = shell_escape(&file_path.to_string_lossy());
        let command = format!("{editor} {quoted}\n");
        tab.terminal.write_bytes(command.as_bytes());
    }

    app.set_output(format!("Opened export in {editor} ({tab_name})."));
}

fn handle_refresh_command(app: &mut App, args: &[String]) {
    if !args.is_empty() {
        app.set_output(REFRESH_USAGE.to_string());
        return;
    }
    request_refresh(app, Some("Refreshed UI."));
}

fn start_command(app: &mut App, name: &str, args: Vec<String>) {
    if app.loading.is_some() {
        app.set_output("Command already running.".to_string());
        return;
    }
    let label = if args.is_empty() {
        format!(":{name}")
    } else {
        format!(":{name} {}", args.join(" "))
    };
    app.loading = Some(label);
    let ctx = CommandContext {
        cwd: app.cwd.clone(),
        repo_root: app.repo_root.clone(),
        workspace_root: app.config.workspace.root.clone(),
        source: CommandSource::Tui,
    };
    let tx = app.event_tx.clone();
    let name = name.to_string();
    std::thread::spawn(move || {
        let result = run_command(&name, &args, &ctx);
        let _ = tx.send(AppEvent::CommandDone { name, args, result });
    });
}

// --- Mouse handling ---

fn handle_mouse_event(app: &mut App, mouse: MouseEvent) {
    if app.loading.is_some() {
        return;
    }
    match mouse.kind {
        MouseEventKind::Down(button) => {
            app.mouse_pressed = Some(button);
        }
        MouseEventKind::Up(_) => {
            app.mouse_pressed = None;
        }
        _ => {}
    }
    let handled = handle_mouse(app, mouse);
    if !handled && app.mode == Mode::Work && !app.command_active && !overlay_visible(app) {
        send_mouse_to_active_terminal(app, mouse);
    }
    if app.mouse_debug {
        log_mouse_debug(app, mouse);
    }
}

fn handle_mouse(app: &mut App, mouse: MouseEvent) -> bool {
    if app.loading.is_some() {
        return false;
    }
    if app.command_active || overlay_visible(app) {
        return false;
    }

    let in_terminal = terminal_cell_from_mouse(app, &mouse).is_some();
    let mouse_mode = active_terminal_ref(app)
        .map(|terminal| terminal.mouse_protocol().0)
        .unwrap_or(MouseProtocolMode::None);
    if in_terminal && mouse_mode != MouseProtocolMode::None {
        return false;
    }

    // Scrolling in terminal area (when not in mouse mode)
    if in_terminal
        && matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        )
    {
        if let Some(terminal) = active_terminal_mut(app) {
            let (mode, _) = terminal.mouse_protocol();
            if mode == MouseProtocolMode::None && !terminal.alternate_screen() {
                let delta = match mouse.kind {
                    MouseEventKind::ScrollUp => SCROLL_LINES,
                    MouseEventKind::ScrollDown => -SCROLL_LINES,
                    _ => 0,
                };
                terminal.scroll_lines(delta);
                return true;
            }
        }
    }

    // Text selection
    if in_terminal && mouse_mode == MouseProtocolMode::None {
        if let Some(pos) = terminal_cell_from_mouse(app, &mouse) {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                app.selection.active = true;
                app.selection.selecting = true;
                app.selection.start = Some(pos);
                app.selection.end = Some(pos);
                return true;
            }
            if matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left))
                && app.selection.selecting
            {
                app.selection.end = Some(pos);
                return true;
            }
            if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left))
                && app.selection.selecting
            {
                app.selection.end = Some(pos);
                app.selection.selecting = false;
                if app.selection.start != app.selection.end {
                    let copied = copy_selection(app);
                    clear_selection(app);
                    return copied;
                }
                clear_selection(app);
                return true;
            }
        }
    }

    // Tab bar clicks
    if !matches!(
        mouse.kind,
        MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
    ) {
        return false;
    }

    let in_tab_bar = app.tab_bar_area.is_some_and(|area| {
        mouse.row >= area.y
            && mouse.row < area.y.saturating_add(area.height)
            && mouse.column >= area.x
            && mouse.column < area.x.saturating_add(area.width)
    });
    if !in_tab_bar {
        return false;
    }

    let Some(workspace) = app.active_workspace.as_deref() else {
        return false;
    };
    let clicked = {
        let Some(tabs) = app.tabs.get(workspace) else {
            return false;
        };
        if tabs.tabs.is_empty() {
            return false;
        }
        let mut cursor = 0u16;
        let mut hit = None;
        for (index, tab) in tabs.tabs.iter().enumerate() {
            let label = format!("{}:{}", index + 1, tab_display_label(app, tab));
            let width = label.chars().count() as u16;
            if mouse.column >= cursor && mouse.column < cursor.saturating_add(width) {
                hit = Some(index);
                break;
            }
            cursor = cursor.saturating_add(width);
            if index + 1 < tabs.tabs.len() {
                cursor = cursor.saturating_add(2);
            }
        }
        hit
    };

    if let Some(index) = clicked {
        if let Some(tabs) = app.tabs.get_mut(workspace) {
            tabs.active = index;
        }
        if app.mouse_debug {
            app.set_output(format!(
                "Mouse click: col={} -> tab {}",
                mouse.column,
                index + 1
            ));
        }
        return true;
    }

    false
}

fn send_mouse_to_active_terminal(app: &mut App, mouse: MouseEvent) -> bool {
    let Some(term_mouse) = mouse_event_for_terminal(app, mouse) else {
        return false;
    };
    let pressed = app.mouse_pressed;
    let Some(terminal) = active_terminal_mut(app) else {
        return false;
    };
    let (mode, encoding) = terminal.mouse_protocol();
    let mut term_mouse = term_mouse;
    if matches!(term_mouse.kind, MouseEventKind::Moved)
        && matches!(
            mode,
            MouseProtocolMode::ButtonMotion | MouseProtocolMode::AnyMotion
        )
    {
        if let Some(button) = pressed {
            term_mouse.kind = MouseEventKind::Drag(button);
        }
    }
    if let Some(bytes) = mouse_event_to_bytes(term_mouse, mode, encoding) {
        terminal.write_bytes(&bytes);
        return true;
    }
    false
}

fn log_mouse_debug(app: &mut App, mouse: MouseEvent) {
    let in_terminal = terminal_cell_from_mouse(app, &mouse).is_some();
    let (mode, encoding, alt_screen, term_pos, encoded) = active_terminal_ref(app)
        .map(|terminal| {
            let (mode, encoding) = terminal.mouse_protocol();
            let alt = terminal.alternate_screen();
            let term_pos =
                mouse_event_for_terminal(app, mouse).map(|event| (event.row, event.column));
            let encoded = mouse_event_for_terminal(app, mouse).and_then(|event| {
                mouse_event_to_bytes(event, mode, encoding).map(|bytes| format_bytes(&bytes))
            });
            (mode, encoding, alt, term_pos, encoded)
        })
        .unwrap_or((
            MouseProtocolMode::None,
            MouseProtocolEncoding::Default,
            false,
            None,
            None,
        ));
    let line = format!(
        "mouse {:?} row={} col={} in_term={} mode={:?} enc={:?} alt={} pressed={:?} term_pos={:?} bytes={}",
        mouse.kind,
        mouse.row,
        mouse.column,
        in_terminal,
        mode,
        encoding,
        alt_screen,
        app.mouse_pressed,
        term_pos,
        encoded.unwrap_or_else(|| "-".to_string())
    );
    app.set_output(line.clone());
    append_mouse_log(app, &line);
}

fn append_mouse_log(app: &App, line: &str) {
    let Some(path) = &app.mouse_log_path else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

// --- Workspace/tab management ---

// Clear stale active workspace state when the worktree disappears.
fn prune_missing_active_workspace(app: &mut App, names: &[String]) {
    let Some(active) = app.active_workspace.clone() else {
        return;
    };
    let Some(root) = app.repo_root.as_ref() else {
        return;
    };
    let active_path = workspace_absolute_path(root, &app.config.workspace.root, &active);
    let missing = !active_path.is_dir() || !names.iter().any(|name| name == &active);
    if !missing {
        return;
    }

    app.active_workspace = None;
    app.tabs.remove(&active);
    app.cwd = root.clone();
    let _ = remove_active_workspace(root);
    ensure_manage_mode_without_workspace(app);
}

pub fn set_active_workspace(app: &mut App, name: &str) -> Result<(), String> {
    if app.active_workspace.as_deref() == Some(name) {
        return Ok(());
    }
    let root = match &app.repo_root {
        Some(root) => root.clone(),
        None => return Err("Not inside a git repository.".to_string()),
    };
    let names = list_workspace_names(&root, &app.config.workspace.root);
    prune_missing_active_workspace(app, &names);
    if !names.iter().any(|entry| entry == name) {
        return Err(format!("Workspace '{name}' not found."));
    }
    let workspace_path = workspace_absolute_path(&root, &app.config.workspace.root, name);
    if !workspace_path.is_dir() {
        return Err(format!("Workspace '{name}' path is missing."));
    }
    app.cwd = workspace_path.clone();
    app.active_workspace = Some(name.to_string());
    clear_selection(app);
    let _ = record_active_workspace(&root, &workspace_path);
    app.config = load_config(&root);
    ensure_active_workspace_tabs(app, 24, 80)
}

fn ensure_manage_mode_without_workspace(app: &mut App) {
    if app.active_workspace.is_none() {
        app.mode = Mode::Manage;
    }
}

fn enter_work_mode(app: &mut App) -> bool {
    if app.active_workspace.is_none() {
        app.set_output(NO_ACTIVE_WORKSPACE_HINT.to_string());
        app.mode = Mode::Manage;
        return false;
    }
    app.mode = Mode::Work;
    true
}

fn request_refresh(app: &mut App, message: Option<&str>) {
    app.refresh_requested = true;
    if let Some(message) = message {
        app.set_output(message.to_string());
    }
}

fn open_workspace_overlay(app: &mut App) {
    let root = match &app.repo_root {
        Some(root) => root.clone(),
        None => {
            app.overlay.message = Some("Not inside a git repository.".to_string());
            app.overlay.items.clear();
            app.overlay.selected = 0;
            app.overlay.visible = true;
            return;
        }
    };

    let names = list_workspace_names(&root, &app.config.workspace.root);
    if names.is_empty() {
        app.overlay.message = Some("No workspaces yet.".to_string());
        app.overlay.items.clear();
        app.overlay.selected = 0;
    } else {
        app.overlay.message = None;
        app.overlay.items = names;
        if let Some(active) = &app.active_workspace {
            if let Some(index) = app.overlay.items.iter().position(|name| name == active) {
                app.overlay.selected = index;
            } else {
                app.overlay.selected = 0;
            }
        } else {
            app.overlay.selected = 0;
        }
    }

    app.overlay.visible = true;
}

fn open_tab_overlay(app: &mut App) {
    let Some(workspace) = app.active_workspace.as_deref() else {
        app.set_output("No active workspace.".to_string());
        return;
    };
    let Some(tabs) = app.tabs.get(workspace) else {
        app.set_output("No tabs for active workspace.".to_string());
        return;
    };
    if tabs.tabs.is_empty() {
        app.set_output("No tabs yet.".to_string());
        return;
    }
    app.tab_overlay.items = (0..tabs.tabs.len()).collect();
    app.tab_overlay.selected = tabs.active;
    app.tab_overlay.visible = true;
}

fn open_pr_provider_overlay(app: &mut App, pending: PendingCommand) {
    let providers = pr::provider_names();
    app.prompt_overlay.title = "Agent Provider".to_string();
    if providers.is_empty() {
        app.prompt_overlay.message = Some("No PR providers available.".to_string());
        app.prompt_overlay.items.clear();
        app.prompt_overlay.selected = 0;
    } else {
        app.prompt_overlay.message = None;
        app.prompt_overlay.items = providers;
        app.prompt_overlay.selected = 0;
    }
    app.prompt_overlay.visible = true;
    app.pending_command = Some(pending);
}

pub fn ensure_active_workspace_tabs(app: &mut App, rows: u16, cols: u16) -> Result<(), String> {
    let Some(workspace) = app.active_workspace.clone() else {
        return Ok(());
    };
    if app
        .tabs
        .get(&workspace)
        .map(|tabs| tabs.tabs.is_empty())
        .unwrap_or(true)
    {
        let name = spawn_tab(app, rows, cols, None)?;
        app.set_output(format!("Opened tab: {name}"));
    }
    Ok(())
}

fn create_tab_for_active(
    app: &mut App,
    rows: u16,
    cols: u16,
    name: Option<String>,
) -> Result<String, String> {
    if app.active_workspace.is_none() {
        return Err("No active workspace.".to_string());
    }
    spawn_tab(app, rows, cols, name)
}

fn spawn_tab(app: &mut App, rows: u16, cols: u16, name: Option<String>) -> Result<String, String> {
    let workspace = app
        .active_workspace
        .clone()
        .ok_or_else(|| "No active workspace.".to_string())?;
    let shell = app
        .config
        .terminal
        .command
        .clone()
        .unwrap_or_else(default_shell);
    let args = app.config.terminal.args.clone();
    let session = TerminalSession::spawn(
        app.terminal_seq,
        &shell,
        &args,
        &app.cwd,
        rows,
        cols,
        app.event_tx.clone(),
    )
    .map_err(|err| format!("Failed to start shell: {err}"))?;
    app.terminal_seq = app.terminal_seq.wrapping_add(1);

    let desired_name = match name {
        Some(name) => {
            let name = name.trim().to_string();
            validate_tab_name(&name)?;
            Some(name)
        }
        None => None,
    };

    let tabs = app.tabs.entry(workspace).or_insert_with(WorkspaceTabs::new);
    let (name, explicit_name) = match desired_name {
        Some(name) => {
            if tabs.tabs.iter().any(|tab| tab.name == name) {
                return Err(format!("Tab '{name}' already exists."));
            }
            (name, true)
        }
        None => {
            let name = format!("tab-{}", tabs.next_index);
            tabs.next_index += 1;
            (name, false)
        }
    };
    tabs.tabs.push(WorkspaceTab {
        id: session.id(),
        name: name.clone(),
        explicit_name,
        terminal: session,
    });
    tabs.active = tabs.tabs.len().saturating_sub(1);
    Ok(name)
}

fn tab_next(app: &mut App) {
    let Some(workspace) = app.active_workspace.as_deref() else {
        return;
    };
    if let Some(tabs) = app.tabs.get_mut(workspace) {
        if tabs.tabs.is_empty() {
            return;
        }
        tabs.active = (tabs.active + 1) % tabs.tabs.len();
        clear_selection(app);
    }
}

fn tab_prev(app: &mut App) {
    let Some(workspace) = app.active_workspace.as_deref() else {
        return;
    };
    if let Some(tabs) = app.tabs.get_mut(workspace) {
        if tabs.tabs.is_empty() {
            return;
        }
        if tabs.active == 0 {
            tabs.active = tabs.tabs.len().saturating_sub(1);
        } else {
            tabs.active -= 1;
        }
        clear_selection(app);
    }
}

fn tab_select(app: &mut App, index: usize) {
    let Some(workspace) = app.active_workspace.as_deref() else {
        return;
    };
    if let Some(tabs) = app.tabs.get_mut(workspace) {
        if index < tabs.tabs.len() {
            tabs.active = index;
            clear_selection(app);
        }
    }
}

fn tab_select_by_arg(app: &mut App, arg: &str) {
    if let Ok(index) = arg.parse::<usize>() {
        if index == 0 {
            app.set_output("Tab index starts at 1.".to_string());
        } else {
            tab_select(app, index - 1);
        }
        return;
    }

    let Some(workspace) = app.active_workspace.as_deref() else {
        app.set_output("No active workspace.".to_string());
        return;
    };
    if let Some(tabs) = app.tabs.get_mut(workspace) {
        if let Some(index) = tabs.tabs.iter().position(|tab| tab.name == arg) {
            tabs.active = index;
        } else {
            app.set_output(format!("Tab '{arg}' not found."));
        }
    }
}

fn rename_active_tab(app: &mut App, name: &str) -> Result<(), String> {
    let name = name.trim();
    validate_tab_name(name)?;
    let workspace = app
        .active_workspace
        .as_deref()
        .ok_or_else(|| "No active workspace.".to_string())?;
    let tabs = app
        .tabs
        .get_mut(workspace)
        .ok_or_else(|| "No tabs for active workspace.".to_string())?;
    if tabs.tabs.iter().any(|tab| tab.name == name) {
        return Err(format!("Tab '{name}' already exists."));
    }
    let tab = tabs
        .tabs
        .get_mut(tabs.active)
        .ok_or_else(|| "No active tab.".to_string())?;
    tab.name = name.to_string();
    tab.explicit_name = true;
    Ok(())
}

fn close_active_tab(app: &mut App) -> Result<String, String> {
    let workspace = app
        .active_workspace
        .as_deref()
        .ok_or_else(|| "No active workspace.".to_string())?;
    let tabs = app
        .tabs
        .get_mut(workspace)
        .ok_or_else(|| "No tabs for active workspace.".to_string())?;
    if tabs.tabs.len() <= 1 {
        return Err("Cannot close the last tab.".to_string());
    }
    let removed = tabs.tabs.remove(tabs.active);
    if tabs.active >= tabs.tabs.len() {
        tabs.active = tabs.tabs.len().saturating_sub(1);
    }
    Ok(format!("Closed tab: {}", removed.name))
}

fn close_tab_by_id(app: &mut App, id: u64) {
    let mut target_workspace: Option<String> = None;
    let mut target_index: Option<usize> = None;
    for (workspace, tabs) in &app.tabs {
        if let Some(index) = tabs.tabs.iter().position(|tab| tab.id == id) {
            target_workspace = Some(workspace.clone());
            target_index = Some(index);
            break;
        }
    }
    let (Some(workspace), Some(index)) = (target_workspace, target_index) else {
        return;
    };
    if let Some(tabs) = app.tabs.get_mut(&workspace) {
        tabs.tabs.remove(index);
        if tabs.tabs.is_empty() {
            tabs.active = 0;
        } else if tabs.active >= tabs.tabs.len() {
            tabs.active = tabs.tabs.len() - 1;
        }
    }
    if app.active_workspace.as_deref() == Some(workspace.as_str()) {
        clear_selection(app);
    }
}

fn validate_tab_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Tab name cannot be empty.".to_string());
    }
    if name.chars().any(|ch| ch.is_whitespace()) {
        return Err("Tab name cannot contain spaces.".to_string());
    }
    Ok(())
}

// --- Terminal helpers ---

pub fn active_terminal_mut(app: &mut App) -> Option<&mut TerminalSession> {
    let workspace = app.active_workspace.as_deref()?;
    let tabs = app.tabs.get_mut(workspace)?;
    let index = tabs.active;
    tabs.tabs.get_mut(index).map(|tab| &mut tab.terminal)
}

pub fn active_terminal_ref(app: &App) -> Option<&TerminalSession> {
    let workspace = app.active_workspace.as_deref()?;
    let tabs = app.tabs.get(workspace)?;
    let index = tabs.active;
    tabs.tabs.get(index).map(|tab| &tab.terminal)
}

pub fn active_tab_ref(app: &App) -> Option<&WorkspaceTab> {
    let workspace = app.active_workspace.as_deref()?;
    let tabs = app.tabs.get(workspace)?;
    let index = tabs.active;
    tabs.tabs.get(index)
}

pub fn active_tab_mut(app: &mut App) -> Option<&mut WorkspaceTab> {
    let workspace = app.active_workspace.as_deref()?;
    let tabs = app.tabs.get_mut(workspace)?;
    let index = tabs.active;
    tabs.tabs.get_mut(index)
}

fn process_terminal_output(app: &mut App, id: u64, bytes: &[u8]) {
    for tabs in app.tabs.values_mut() {
        for tab in &mut tabs.tabs {
            if tab.id == id {
                tab.terminal.process_bytes(bytes);
                return;
            }
        }
    }
}

pub fn terminal_cell_from_mouse(app: &App, mouse: &MouseEvent) -> Option<CellPos> {
    let area = app.terminal_area?;
    if mouse.row < area.y
        || mouse.row >= area.y.saturating_add(area.height)
        || mouse.column < area.x
        || mouse.column >= area.x.saturating_add(area.width)
    {
        return None;
    }
    Some(CellPos {
        row: mouse.row.saturating_sub(area.y),
        col: mouse.column.saturating_sub(area.x),
    })
}

fn mouse_event_for_terminal(app: &App, mouse: MouseEvent) -> Option<MouseEvent> {
    let pos = terminal_cell_from_mouse(app, &mouse)?;
    Some(MouseEvent {
        row: pos.row,
        column: pos.col,
        ..mouse
    })
}

// --- Selection helpers ---

fn clear_selection(app: &mut App) {
    app.selection = super::state::SelectionState::default();
}

pub fn normalized_selection(app: &App, rows: u16, cols: u16) -> Option<(CellPos, CellPos)> {
    if !app.selection.active {
        return None;
    }
    let mut start = app.selection.start?;
    let mut end = app.selection.end.unwrap_or(start);
    start.row = start.row.min(rows.saturating_sub(1));
    end.row = end.row.min(rows.saturating_sub(1));
    start.col = start.col.min(cols.saturating_sub(1));
    end.col = end.col.min(cols.saturating_sub(1));
    if (end.row, end.col) < (start.row, start.col) {
        std::mem::swap(&mut start, &mut end);
    }
    Some((start, end))
}

pub fn selection_ranges(app: &App, rows: u16, cols: u16) -> Option<Vec<Vec<(u16, u16)>>> {
    let (start, end) = normalized_selection(app, rows, cols)?;
    let mut ranges = vec![Vec::new(); rows as usize];
    for row in start.row..=end.row {
        let row_start = if row == start.row { start.col } else { 0 };
        let row_end = if row == end.row {
            end.col
        } else {
            cols.saturating_sub(1)
        };
        let end_exclusive = row_end.saturating_add(1);
        if let Some(row_ranges) = ranges.get_mut(row as usize) {
            row_ranges.push((row_start, end_exclusive));
        }
    }
    Some(ranges)
}

pub fn active_search_range(app: &App) -> Option<(u16, u16, u16)> {
    app.search
        .matches
        .get(app.search.active_index)
        .map(|match_| (match_.row, match_.start, match_.end))
}

fn copy_selection(app: &mut App) -> bool {
    let Some(terminal) = active_terminal_ref(app) else {
        app.set_output("No active workspace.".to_string());
        return true;
    };
    let rows = terminal.rows();
    let cols = terminal.cols();
    let Some((start, end)) = normalized_selection(app, rows, cols) else {
        app.set_output("No selection.".to_string());
        return true;
    };
    let text = terminal.contents_between(start.row, start.col, end.row, end.col);
    if text.trim().is_empty() {
        app.set_output("Selection is empty.".to_string());
        return true;
    }
    match Clipboard::new() {
        Ok(mut clipboard) => {
            if clipboard.set_text(text).is_ok() {
                app.set_output("Copied selection.".to_string());
            } else {
                app.set_output("Failed to copy selection.".to_string());
            }
        }
        Err(err) => {
            app.set_output(format!("Clipboard unavailable: {err}"));
        }
    }
    true
}

pub fn overlay_visible(app: &App) -> bool {
    app.overlay.visible || app.tab_overlay.visible || app.prompt_overlay.visible
}

pub fn tab_display_label(_app: &App, tab: &WorkspaceTab) -> String {
    if tab.explicit_name {
        return truncate_label(&tab.name, MAX_TAB_LABEL_LEN);
    }
    let title = tab.terminal.title().trim();
    if title.is_empty() {
        truncate_label(&tab.name, MAX_TAB_LABEL_LEN)
    } else {
        let title = simplify_title(title);
        truncate_label(&title, MAX_TAB_LABEL_LEN)
    }
}

// --- Utility functions ---

fn format_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn simplify_title(title: &str) -> String {
    let mut cleaned = title.trim();
    if let Some((head, _)) = cleaned.split_once(" - ") {
        cleaned = head.trim();
    }
    if let Some(idx) = cleaned.rfind(&['/', '\\'][..]) {
        let tail = cleaned[idx + 1..].trim();
        if !tail.is_empty() {
            return tail.to_string();
        }
    }
    cleaned.to_string()
}

fn truncate_label(label: &str, max_len: usize) -> String {
    let len = label.chars().count();
    if len <= max_len {
        return label.to_string();
    }
    if max_len <= 3 {
        return label.chars().take(max_len).collect();
    }
    let keep = max_len - 3;
    let mut out: String = label.chars().take(keep).collect();
    out.push_str("...");
    out
}

fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

fn find_editor_binary() -> Option<String> {
    for name in ["vim", "vi"] {
        if find_executable(name).is_some() {
            return Some(name.to_string());
        }
    }
    None
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() && is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|meta| meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

fn shell_escape(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' || ch == '/')
    {
        return value.to_string();
    }
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            escaped.push_str("'\\''");
        } else {
            escaped.push(ch);
        }
    }
    escaped.push('\'');
    escaped
}

/// Compute search matches for the current terminal view.
pub fn compute_search_matches(
    query: &str,
    terminal: &TerminalSession,
    rows: u16,
    cols: u16,
) -> (Vec<SearchMatch>, Vec<Vec<(u16, u16)>>) {
    let query = query.trim();
    if query.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let lines = terminal.visible_rows_text(rows, cols);
    let needle: Vec<char> = query.to_lowercase().chars().collect();
    let mut matches = Vec::new();
    let mut ranges = vec![Vec::new(); rows as usize];

    for (row_idx, line) in lines.iter().enumerate() {
        let line_chars: Vec<char> = line.to_lowercase().chars().collect();
        if needle.is_empty() || line_chars.len() < needle.len() {
            continue;
        }
        for col in 0..=line_chars.len() - needle.len() {
            if line_chars[col..col + needle.len()] == needle {
                let start = col as u16;
                let end = (col + needle.len()) as u16;
                matches.push(SearchMatch {
                    row: row_idx as u16,
                    start,
                    end,
                });
                if let Some(row_ranges) = ranges.get_mut(row_idx) {
                    row_ranges.push((start, end));
                }
            }
        }
    }

    (matches, ranges)
}
