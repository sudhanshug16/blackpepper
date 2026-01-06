use crossterm::event::{KeyCode, KeyEvent};

use crate::config::save_user_agent_provider;
use crate::providers::agent;
use crate::workspaces::list_workspace_names;

use super::workspace::set_active_workspace;
use crate::app::state::{App, PendingCommand};

pub(super) fn handle_overlay_key(app: &mut App, key: KeyEvent) {
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

pub(super) fn handle_prompt_overlay_key(app: &mut App, key: KeyEvent) {
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
                    super::command::start_command(app, &pending.name, pending.args);
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

pub(super) fn open_workspace_overlay(app: &mut App) {
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

pub(super) fn open_agent_provider_overlay(app: &mut App, pending: PendingCommand) {
    let providers = agent::provider_names();
    app.prompt_overlay.title = "Agent Provider".to_string();
    if providers.is_empty() {
        app.prompt_overlay.message = Some("No agent providers available.".to_string());
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
