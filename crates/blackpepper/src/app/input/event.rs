use termwiz::input::{InputEvent, KeyCode, KeyEvent, Modifiers};

use crate::commands::CommandPhase;
use crate::events::AppEvent;
use crate::keymap::matches_chord;

use super::command::handle_command_input;
use super::overlay::{handle_overlay_key, handle_prompt_overlay_key, open_workspace_overlay};
use super::terminal::process_terminal_output;
use super::workspace::{
    active_terminal_mut, close_session_by_id, ensure_manage_mode_without_workspace,
    enter_work_mode, set_active_workspace,
};
use crate::app::state::{App, Mode};

/// Main event dispatcher.
pub fn handle_event(app: &mut App, event: AppEvent) {
    match event {
        AppEvent::RawInput(bytes) => handle_raw_input(app, bytes),
        AppEvent::InputFlush => flush_input(app),
        AppEvent::PtyOutput(id, bytes) => {
            process_terminal_output(app, id, &bytes);
        }
        AppEvent::PtyExit(id) => close_session_by_id(app, id),
        AppEvent::Resize => {}
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
                    if (subcommand == "create"
                        || subcommand == "from-branch"
                        || subcommand == "from-pr")
                        && result.ok
                    {
                        if let Some(name) = result.data.as_deref() {
                            if set_active_workspace(app, name).is_ok() {
                                app.set_output(format!("Active workspace: {name}"));
                            }
                        }
                        enter_work_mode(app);
                    }
                    if subcommand == "rename" && result.ok {
                        if let Some(name) = result.data.as_deref() {
                            if let Some(old) = app.active_workspace.clone() {
                                if let Some(session) = app.sessions.remove(&old) {
                                    app.sessions.insert(name.to_string(), session);
                                }
                                app.active_workspace = None;
                            }
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
    if app.mode == Mode::Work {
        return;
    }

    let mods = key.modifiers.remove_positional_mods();
    if app.loading.is_some() {
        return;
    }
    if app.command_overlay.visible {
        if key.key == KeyCode::Escape && mods == Modifiers::NONE {
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
        if matches_chord(&key, chord) {
            enter_work_mode(app);
            return;
        }
    }

    // Switch workspace chord
    if let Some(chord) = &app.switch_chord {
        if matches_chord(&key, chord) {
            open_workspace_overlay(app);
            return;
        }
    }

    // Manage mode: open command bar with ':'
    if key.key == KeyCode::Char(':') {
        super::command::open_command(app);
        return;
    }

    // Manage mode: quit with 'q'
    if key.key == KeyCode::Char('q') && mods == Modifiers::NONE {
        app.should_quit = true;
        return;
    }

    // Manage mode: escape returns to work mode
    if key.key == KeyCode::Escape {
        enter_work_mode(app);
    }
}

fn handle_raw_input(app: &mut App, bytes: Vec<u8>) {
    if bytes.is_empty() {
        return;
    }

    match app.mode {
        Mode::Manage => {
            use crate::input::MatchedChord;
            let (filtered, matched) = app.input_decoder.consume_work_bytes(&bytes);
            let events = app.input_decoder.parse_manage_vec(&filtered, true);
            for event in events {
                handle_input_event(app, event);
            }
            // Both toggle and switch chords return to work mode from manage mode
            if matched != MatchedChord::None {
                enter_work_mode(app);
            }
        }
        Mode::Work => {
            use crate::input::MatchedChord;
            let (out, matched) = app.input_decoder.consume_work_bytes(&bytes);
            if let Some(terminal) = active_terminal_mut(app) {
                if !out.is_empty() {
                    terminal.write_bytes(&out);
                }
            }
            match matched {
                MatchedChord::Toggle => {
                    app.set_mode(Mode::Manage);
                }
                MatchedChord::Switch => {
                    app.set_mode(Mode::Manage);
                    open_workspace_overlay(app);
                }
                MatchedChord::None => {}
            }
        }
    }
}

fn flush_input(app: &mut App) {
    match app.mode {
        Mode::Manage => {
            let buffered = app.input_decoder.flush_work();
            let events = app.input_decoder.parse_manage_vec(&buffered, false);
            for event in events {
                handle_input_event(app, event);
            }
            let events = app.input_decoder.flush_manage_vec();
            for event in events {
                handle_input_event(app, event);
            }
        }
        Mode::Work => {
            let out = app.input_decoder.flush_work();
            if let Some(terminal) = active_terminal_mut(app) {
                if !out.is_empty() {
                    terminal.write_bytes(&out);
                }
            }
        }
    }
}

fn handle_input_event(app: &mut App, event: InputEvent) {
    match event {
        InputEvent::Key(key) => handle_key(app, key),
        InputEvent::Paste(paste) => {
            if app.command_active {
                app.command_input.push_str(&paste);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::handle_event;
    use crate::app::state::{App, Mode, WorkspaceSession};
    use crate::commands::CommandResult;
    use crate::events::AppEvent;
    use crate::terminal::TerminalSession;
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{mpsc, Mutex};
    use tempfile::TempDir;

    static STATE_LOCK: Mutex<()> = Mutex::new(());

    struct DirGuard {
        previous: PathBuf,
    }

    impl Drop for DirGuard {
        fn drop(&mut self) {
            let _ = env::set_current_dir(&self.previous);
        }
    }

    fn enter_dir(path: &Path) -> DirGuard {
        let previous = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        env::set_current_dir(path).expect("set current dir");
        DirGuard { previous }
    }

    fn with_state_path<T>(path: &Path, action: impl FnOnce() -> T) -> T {
        let _guard = STATE_LOCK.lock().expect("state lock");
        let key = "BLACKPEPPER_STATE_PATH";
        let previous = env::var(key).ok();
        env::set_var(key, path);
        let result = action();
        match previous {
            Some(value) => env::set_var(key, value),
            None => env::remove_var(key),
        }
        result
    }

    fn run_git_cmd(args: &[&str], cwd: &Path) {
        let status = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "Test User")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test User")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .expect("run git");
        assert!(status.success(), "git {:?} failed", args);
    }

    fn init_repo() -> TempDir {
        let repo = TempDir::new().expect("temp repo");
        run_git_cmd(&["init", "-b", "main"], repo.path());
        fs::write(repo.path().join("README.md"), "hello").expect("write file");
        run_git_cmd(&["add", "."], repo.path());
        run_git_cmd(&["commit", "-m", "init"], repo.path());
        repo
    }

    fn add_worktree(repo: &Path, path: &Path, branch: &str) {
        run_git_cmd(
            &["worktree", "add", "-b", branch, path.to_str().unwrap()],
            repo,
        );
    }

    fn spawn_stub_session(tx: mpsc::Sender<AppEvent>, cwd: &Path) -> TerminalSession {
        let (shell, args) = if cfg!(windows) {
            ("cmd", vec!["/C".to_string(), "exit 0".to_string()])
        } else {
            ("sh", vec!["-c".to_string(), "exit 0".to_string()])
        };
        TerminalSession::spawn(
            1,
            "test",
            shell,
            &args,
            cwd,
            24,
            80,
            (255, 255, 255),
            (0, 0, 0),
            tx,
        )
        .expect("spawn stub session")
    }

    #[test]
    fn workspace_rename_switches_active_workspace() {
        let repo = init_repo();
        let repo_root = fs::canonicalize(repo.path()).unwrap_or_else(|_| repo.path().to_path_buf());
        let workspace_root = repo_root.join(".blackpepper/workspaces");
        fs::create_dir_all(&workspace_root).expect("workspace root");
        let new_path = workspace_root.join("new");
        add_worktree(&repo_root, &new_path, "new");
        let state_path = repo_root.join("state.toml");
        let config_path = repo_root
            .join(".config")
            .join("blackpepper")
            .join("config.toml");
        fs::create_dir_all(config_path.parent().expect("config dir")).expect("config dir");
        fs::write(
            &config_path,
            "[workspace]\nroot = \".blackpepper/workspaces\"\n",
        )
        .expect("write config");
        let _guard = enter_dir(&repo_root);

        with_state_path(&state_path, || {
            let (tx, _rx) = mpsc::channel();
            let mut app = App::new(tx.clone());
            app.repo_root = Some(repo_root.clone());
            app.cwd = repo_root.clone();
            app.active_workspace = Some("old".to_string());
            let session = spawn_stub_session(tx, repo.path());
            app.sessions
                .insert("old".to_string(), WorkspaceSession { terminal: session });

            let result = CommandResult {
                ok: true,
                message: "Renamed workspace".to_string(),
                data: Some("new".to_string()),
            };
            let event = AppEvent::CommandDone {
                name: "workspace".to_string(),
                args: vec!["rename".to_string(), "new".to_string()],
                result,
            };
            handle_event(&mut app, event);

            assert_eq!(app.active_workspace.as_deref(), Some("new"));
            assert!(app.sessions.contains_key("new"));
            assert!(!app.sessions.contains_key("old"));
        });
    }

    #[test]
    fn workspace_create_enters_work_mode() {
        let repo = init_repo();
        let repo_root = fs::canonicalize(repo.path()).unwrap_or_else(|_| repo.path().to_path_buf());
        let workspace_root = repo_root.join(".blackpepper/workspaces");
        fs::create_dir_all(&workspace_root).expect("workspace root");
        let ws_path = workspace_root.join("bp.otter");
        add_worktree(&repo_root, &ws_path, "bp.otter");
        let state_path = repo_root.join("state.toml");
        let config_path = repo_root
            .join(".config")
            .join("blackpepper")
            .join("config.toml");
        fs::create_dir_all(config_path.parent().expect("config dir")).expect("config dir");
        fs::write(
            &config_path,
            "[workspace]\nroot = \".blackpepper/workspaces\"\n",
        )
        .expect("write config");
        let _guard = enter_dir(&repo_root);

        with_state_path(&state_path, || {
            let (tx, _rx) = mpsc::channel();
            let mut app = App::new(tx.clone());
            app.repo_root = Some(repo_root.clone());
            app.cwd = repo_root.clone();
            app.set_mode(Mode::Manage);

            let result = CommandResult {
                ok: true,
                message: "Created workspace".to_string(),
                data: Some("bp.otter".to_string()),
            };
            let event = AppEvent::CommandDone {
                name: "workspace".to_string(),
                args: vec!["create".to_string()],
                result,
            };
            handle_event(&mut app, event);

            assert_eq!(app.active_workspace.as_deref(), Some("bp.otter"));
            assert_eq!(app.mode, Mode::Work, "should enter work mode after create");
        });
    }

    #[test]
    fn workspace_rename_enters_work_mode() {
        let repo = init_repo();
        let repo_root = fs::canonicalize(repo.path()).unwrap_or_else(|_| repo.path().to_path_buf());
        let workspace_root = repo_root.join(".blackpepper/workspaces");
        fs::create_dir_all(&workspace_root).expect("workspace root");
        let new_path = workspace_root.join("new");
        add_worktree(&repo_root, &new_path, "new");
        let state_path = repo_root.join("state.toml");
        let config_path = repo_root
            .join(".config")
            .join("blackpepper")
            .join("config.toml");
        fs::create_dir_all(config_path.parent().expect("config dir")).expect("config dir");
        fs::write(
            &config_path,
            "[workspace]\nroot = \".blackpepper/workspaces\"\n",
        )
        .expect("write config");
        let _guard = enter_dir(&repo_root);

        with_state_path(&state_path, || {
            let (tx, _rx) = mpsc::channel();
            let mut app = App::new(tx.clone());
            app.repo_root = Some(repo_root.clone());
            app.cwd = repo_root.clone();
            app.active_workspace = Some("old".to_string());
            app.set_mode(Mode::Manage);
            let session = spawn_stub_session(tx, repo.path());
            app.sessions
                .insert("old".to_string(), WorkspaceSession { terminal: session });

            let result = CommandResult {
                ok: true,
                message: "Renamed workspace".to_string(),
                data: Some("new".to_string()),
            };
            let event = AppEvent::CommandDone {
                name: "workspace".to_string(),
                args: vec!["rename".to_string(), "new".to_string()],
                result,
            };
            handle_event(&mut app, event);

            assert_eq!(app.active_workspace.as_deref(), Some("new"));
            assert_eq!(app.mode, Mode::Work, "should enter work mode after rename");
        });
    }
}
