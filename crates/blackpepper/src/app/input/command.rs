use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent};

use crate::commands::{
    complete_command_input, parse_command, run_command, run_command_with_output, CommandContext,
    CommandOutput, CommandSource,
};
use crate::events::AppEvent;
use crate::workspaces::list_workspace_names;

use super::overlay::{open_pr_provider_overlay, open_tab_overlay, open_workspace_overlay};
use crate::app::state::{App, PendingCommand};
use super::utils::{find_editor_binary, shell_escape};
use super::workspace::{
    active_tab_mut, close_active_tab, create_tab_for_active, enter_work_mode,
    prune_missing_active_workspace, rename_active_tab, request_refresh, set_active_workspace,
    spawn_tab, tab_next, tab_prev, tab_select_by_arg,
};
use super::NO_ACTIVE_WORKSPACE_HINT;

const REFRESH_USAGE: &str = "Usage: :refresh";

pub(super) fn handle_command_input(app: &mut App, key: KeyEvent) {
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

pub(super) fn open_command(app: &mut App) {
    app.command_active = true;
    app.command_input = ":".to_string();
}

pub(super) fn start_command(app: &mut App, name: &str, args: Vec<String>) {
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
        let name_for_output = name.clone();
        let mut on_output = |event: CommandOutput| match event {
            CommandOutput::Chunk(chunk) => {
                let _ = tx.send(AppEvent::CommandOutput {
                    name: name_for_output.clone(),
                    chunk,
                });
            }
            CommandOutput::PhaseComplete(phase) => {
                let _ = tx.send(AppEvent::CommandPhaseComplete { phase });
            }
        };
        let result = run_command_with_output(&name, &args, &ctx, &mut on_output);
        let _ = tx.send(AppEvent::CommandDone { name, args, result });
    });
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
