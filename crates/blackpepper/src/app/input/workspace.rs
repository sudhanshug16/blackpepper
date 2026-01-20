use crate::config::load_config;
use crate::keymap::parse_key_chord;
use crate::repo_status::RepoStatusSignal;
use crate::state::{
    ensure_workspace_ports, record_active_workspace, remove_active_workspace, workspace_port_env,
};
use crate::terminal::TerminalSession;
use crate::tmux;
use crate::workspaces::{list_workspace_names, workspace_absolute_path};

use crate::app::state::{App, Mode, WorkspaceSession};

use super::NO_ACTIVE_WORKSPACE_HINT;

/// Expand `$VAR` and `${VAR}` references in a string using the provided env vars.
fn expand_env_vars(template: &str, env: &[(String, String)]) -> String {
    let mut result = template.to_string();
    for (key, value) in env {
        // Replace ${VAR} and $VAR patterns
        result = result.replace(&format!("${{{key}}}"), value);
        result = result.replace(&format!("${key}"), value);
    }
    result
}

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
    app.sessions.remove(&active);
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
    let _ = record_active_workspace(&root, &workspace_path);
    app.config = load_config(&root);
    app.toggle_chord = parse_key_chord(&app.config.keymap.toggle_mode);
    app.switch_chord = parse_key_chord(&app.config.keymap.switch_workspace);
    app.input_decoder
        .update_chords(app.toggle_chord.clone(), app.switch_chord.clone());
    request_repo_status(app);
    ensure_active_workspace_session(app, 24, 80)
}

pub(super) fn cycle_workspace(app: &mut App) {
    let root = match &app.repo_root {
        Some(root) => root.clone(),
        None => {
            app.set_output("Not inside a git repository.".to_string());
            return;
        }
    };
    let names = list_workspace_names(&root, &app.config.workspace.root);
    prune_missing_active_workspace(app, &names);
    if names.is_empty() {
        app.set_output("No workspaces yet.".to_string());
        return;
    }

    let next = match app.active_workspace.as_deref() {
        Some(active) => match names.iter().position(|name| name == active) {
            Some(index) => names[(index + 1) % names.len()].clone(),
            None => names[0].clone(),
        },
        None => names[0].clone(),
    };

    match set_active_workspace(app, &next) {
        Ok(()) => {
            app.set_output(format!("Active workspace: {next}"));
            enter_work_mode(app);
        }
        Err(err) => app.set_output(err),
    }
}

pub(super) fn ensure_manage_mode_without_workspace(app: &mut App) {
    if app.active_workspace.is_none() {
        app.set_mode(Mode::Manage);
    }
}

pub(super) fn enter_work_mode(app: &mut App) -> bool {
    if app.active_workspace.is_none() {
        app.set_output(NO_ACTIVE_WORKSPACE_HINT.to_string());
        app.set_mode(Mode::Manage);
        return false;
    }
    let (rows, cols) = app
        .terminal_area
        .map(|area| (area.height, area.width))
        .unwrap_or((24, 80));
    if let Err(err) = ensure_active_workspace_session(app, rows, cols) {
        app.set_output(err);
        app.set_mode(Mode::Manage);
        return false;
    }
    app.set_mode(Mode::Work);
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

pub fn ensure_active_workspace_session(app: &mut App, rows: u16, cols: u16) -> Result<(), String> {
    let Some(workspace) = app.active_workspace.clone() else {
        return Ok(());
    };
    if app.sessions.contains_key(&workspace) {
        return Ok(());
    }
    spawn_workspace_session(app, &workspace, rows, cols)
}

fn spawn_workspace_session(
    app: &mut App,
    workspace: &str,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    let Some(repo_root) = app.repo_root.as_ref() else {
        return Err("Not inside a git repository.".to_string());
    };
    let session_name = tmux::session_name(repo_root, workspace);
    let tabs = tmux::resolve_tabs(&app.config.tmux);
    let setup_tab = tmux::setup_command_args(&app.config.workspace.setup_scripts).map(|command| {
        tmux::SetupTab {
            name: tmux::SETUP_TMUX_TAB.to_string(),
            command,
        }
    });
    let workspace_ports = ensure_workspace_ports(&app.cwd)
        .map_err(|err| format!("Failed to allocate workspace ports: {err}"))?;
    let mut env = workspace_port_env(workspace_ports);
    env.push((
        "BLACKPEPPER_REPO_ROOT".to_string(),
        repo_root.to_string_lossy().to_string(),
    ));
    // Add workspace-defined env vars with variable expansion
    for (key, value) in &app.config.workspace.env {
        let expanded = expand_env_vars(value, &env);
        env.push((key.clone(), expanded));
    }
    tmux::ensure_session_layout(
        &app.config.tmux,
        &session_name,
        &app.cwd,
        setup_tab,
        &tabs,
        &env,
    )
    .map_err(|err| format!("Failed to prepare tmux session: {err}"))?;
    let (command, args) = tmux::client_command(&app.config.tmux, &session_name, &app.cwd);
    let session = TerminalSession::spawn(
        app.terminal_seq,
        workspace,
        &command,
        &args,
        &app.cwd,
        rows,
        cols,
        app.config.ui.foreground,
        app.config.ui.background,
        app.event_tx.clone(),
    )
    .map_err(|err| format!("Failed to start tmux: {err}"))?;
    app.terminal_seq = app.terminal_seq.wrapping_add(1);
    app.sessions.insert(
        workspace.to_string(),
        WorkspaceSession { terminal: session },
    );
    Ok(())
}

pub(super) fn close_session_by_id(app: &mut App, id: u64) {
    let target = app
        .sessions
        .iter()
        .find(|(_, session)| session.terminal.id() == id)
        .map(|(name, _)| name.clone());
    let Some(workspace) = target else {
        return;
    };
    app.sessions.remove(&workspace);
    if app.active_workspace.as_deref() == Some(workspace.as_str()) {
        app.set_mode(Mode::Manage);
        app.set_output("tmux session exited.".to_string());
    }
}

pub fn active_terminal_mut(app: &mut App) -> Option<&mut TerminalSession> {
    let workspace = app.active_workspace.as_deref()?;
    app.sessions
        .get_mut(workspace)
        .map(|session| &mut session.terminal)
}

#[cfg(test)]
mod tests {
    use super::expand_env_vars;

    #[test]
    fn expand_env_vars_replaces_dollar_var() {
        let env = vec![
            ("PORT".to_string(), "3000".to_string()),
            ("HOST".to_string(), "localhost".to_string()),
        ];
        assert_eq!(
            expand_env_vars("http://$HOST:$PORT", &env),
            "http://localhost:3000"
        );
    }

    #[test]
    fn expand_env_vars_replaces_brace_var() {
        let env = vec![
            ("PORT".to_string(), "8080".to_string()),
            ("ROOT".to_string(), "/app".to_string()),
        ];
        assert_eq!(
            expand_env_vars("${ROOT}/bin:${PORT}", &env),
            "/app/bin:8080"
        );
    }

    #[test]
    fn expand_env_vars_mixed_syntax() {
        let env = vec![
            ("A".to_string(), "alpha".to_string()),
            ("B".to_string(), "beta".to_string()),
        ];
        assert_eq!(expand_env_vars("$A and ${B}", &env), "alpha and beta");
    }

    #[test]
    fn expand_env_vars_no_match_left_alone() {
        let env = vec![("FOO".to_string(), "bar".to_string())];
        assert_eq!(
            expand_env_vars("$BAZ stays ${QUX}", &env),
            "$BAZ stays ${QUX}"
        );
    }

    #[test]
    fn expand_env_vars_empty_env() {
        let env: Vec<(String, String)> = vec![];
        assert_eq!(expand_env_vars("$FOO ${BAR}", &env), "$FOO ${BAR}");
    }

    #[test]
    fn expand_env_vars_no_recursive_expansion() {
        // Expansion is not recursive - if a value contains $VAR, it won't be expanded
        let env = vec![
            ("PORT".to_string(), "3000".to_string()),
            ("URL".to_string(), "http://localhost:$PORT".to_string()),
        ];
        // $URL expands to the literal value, $PORT within is NOT expanded
        assert_eq!(expand_env_vars("$URL", &env), "http://localhost:$PORT");
    }

    #[test]
    fn expand_env_vars_blackpepper_repo_root() {
        let env = vec![
            (
                "BLACKPEPPER_REPO_ROOT".to_string(),
                "/home/user/project".to_string(),
            ),
            ("BLACKPEPPER_PORT_0".to_string(), "4000".to_string()),
        ];
        assert_eq!(
            expand_env_vars(
                "cd $BLACKPEPPER_REPO_ROOT && npm start --port=$BLACKPEPPER_PORT_0",
                &env
            ),
            "cd /home/user/project && npm start --port=4000"
        );
    }
}
