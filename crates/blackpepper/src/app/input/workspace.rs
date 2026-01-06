use crate::config::load_config;
use crate::state::{record_active_workspace, remove_active_workspace};
use crate::terminal::TerminalSession;
use crate::workspaces::{list_workspace_names, workspace_absolute_path};

use super::terminal::clear_selection;
use super::utils::{default_shell, simplify_title, truncate_label};
use super::NO_ACTIVE_WORKSPACE_HINT;
use crate::app::state::{App, Mode, WorkspaceTab, WorkspaceTabs, MAX_TAB_LABEL_LEN};
use crate::repo_status::RepoStatusSignal;

pub(super) fn prune_missing_active_workspace(app: &mut App, names: &[String]) {
    let Some(root) = app.repo_root.as_ref() else {
        return;
    };
    let Some(active) = app.active_workspace.clone() else {
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

pub(super) fn set_active_workspace(app: &mut App, name: &str) -> Result<(), String> {
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
    request_repo_status(app);
    ensure_active_workspace_tabs(app, 24, 80)
}

pub(super) fn ensure_manage_mode_without_workspace(app: &mut App) {
    if app.active_workspace.is_none() {
        app.mode = Mode::Manage;
    }
}

pub(super) fn enter_work_mode(app: &mut App) -> bool {
    if app.active_workspace.is_none() {
        app.set_output(NO_ACTIVE_WORKSPACE_HINT.to_string());
        app.mode = Mode::Manage;
        return false;
    }
    app.mode = Mode::Work;
    true
}

pub(super) fn request_refresh(app: &mut App, message: Option<&str>) {
    app.refresh_requested = true;
    request_repo_status(app);
    if let Some(message) = message {
        app.set_output(message.to_string());
    }
}

pub(super) fn request_repo_status(app: &App) {
    if let Some(tx) = app.repo_status_tx.as_ref() {
        let _ = tx.send(RepoStatusSignal::Request(app.cwd.clone()));
    }
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

pub(super) fn create_tab_for_active(
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

pub(super) fn spawn_tab(
    app: &mut App,
    rows: u16,
    cols: u16,
    name: Option<String>,
) -> Result<String, String> {
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

pub(super) fn tab_next(app: &mut App) {
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

pub(super) fn tab_prev(app: &mut App) {
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

pub(super) fn tab_select(app: &mut App, index: usize) {
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

pub(super) fn tab_select_by_arg(app: &mut App, arg: &str) {
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

pub(super) fn rename_active_tab(app: &mut App, name: &str) -> Result<(), String> {
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

pub(super) fn close_active_tab(app: &mut App) -> Result<String, String> {
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

pub(super) fn close_tab_by_id(app: &mut App, id: u64) {
    let mut target_workspace: Option<String> = None;
    let mut target_index: Option<usize> = None;
    for (workspace, tabs) in &app.tabs {
        for (index, tab) in tabs.tabs.iter().enumerate() {
            if tab.id == id {
                target_workspace = Some(workspace.clone());
                target_index = Some(index);
                break;
            }
        }
        if target_workspace.is_some() {
            break;
        }
    }

    let Some(workspace) = target_workspace else {
        return;
    };
    let Some(index) = target_index else {
        return;
    };

    if let Some(tabs) = app.tabs.get_mut(&workspace) {
        if index >= tabs.tabs.len() {
            return;
        }
        tabs.tabs.remove(index);
        if tabs.tabs.is_empty() {
            app.tabs.remove(&workspace);
        } else if tabs.active >= tabs.tabs.len() {
            tabs.active = tabs.tabs.len().saturating_sub(1);
        }
    }
}

pub(super) fn validate_tab_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Tab name cannot be empty.".to_string());
    }
    if name.len() > 64 {
        return Err("Tab name is too long (max 64).".to_string());
    }
    if name.contains(['/', '\\', ':']) {
        return Err("Tab name cannot contain '/', '\\', or ':'.".to_string());
    }
    Ok(())
}

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
