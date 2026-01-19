//! Application runner and event loop.
//!
//! Handles terminal setup/teardown and the main event loop.
//! Events are read from an mpsc channel and dispatched to handlers.

use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

#[cfg(not(unix))]
use crossterm::event;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::config::load_config;
use crate::events::AppEvent;
use crate::git::resolve_repo_root;
use crate::input::InputDecoder;
use crate::keymap::parse_key_chord;
use crate::repo_status::{spawn_repo_status_worker, RepoStatus, RepoStatusSignal};
use crate::state::{get_active_workspace, load_state, remove_active_workspace};
use crate::terminal::InputModes;
use crate::workspaces::{list_workspace_names, prune_stale_workspaces, workspace_name_from_path};

use super::state::{App, CommandOverlay, Mode, PromptOverlay, WorkspaceOverlay};

/// Entry point: set up terminal and run the event loop.
pub fn run() -> io::Result<()> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal);

    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// Main event loop: process events until quit.
fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let (event_tx, event_rx) = mpsc::channel::<AppEvent>();
    let mut app = App::new(event_tx.clone());
    spawn_input_thread(event_tx.clone());
    terminal.clear()?;
    terminal.draw(|frame| super::render::render(&mut app, frame))?;
    flush_pending_input_modes(terminal, &mut app)?;

    while !app.should_quit {
        let event = match event_rx.recv() {
            Ok(event) => event,
            Err(_) => break,
        };
        super::input::handle_event(&mut app, event);
        // Drain any pending events before redraw
        while let Ok(event) = event_rx.try_recv() {
            super::input::handle_event(&mut app, event);
        }

        if app.refresh_requested {
            terminal.clear()?;
            app.refresh_requested = false;
        }

        flush_pending_input_modes(terminal, &mut app)?;
        terminal.draw(|frame| super::render::render(&mut app, frame))?;
    }
    Ok(())
}

/// Spawn a thread to read raw terminal input bytes.
fn spawn_input_thread(sender: Sender<AppEvent>) {
    std::thread::spawn(move || {
        let mut last_size: Option<(u16, u16)> = None;
        let mut pending_flush = false;
        loop {
            let mut sent = false;
            match read_raw_bytes(Duration::from_millis(25)) {
                Ok(Some(bytes)) => {
                    pending_flush = true;
                    sent = sender.send(AppEvent::RawInput(bytes)).is_ok();
                }
                Ok(None) => {
                    if pending_flush {
                        pending_flush = false;
                        sent = sender.send(AppEvent::InputFlush).is_ok();
                    }
                }
                Err(_) => break,
            }

            if !sent && !sync_resize(&sender, &mut last_size) {
                break;
            }
        }
    });
}

impl App {
    /// Create a new App instance with loaded config and state.
    pub fn new(event_tx: Sender<AppEvent>) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut repo_root = resolve_repo_root(&cwd);
        let config_root = repo_root.as_deref().unwrap_or(&cwd);
        let config = load_config(config_root);
        let toggle_chord = parse_key_chord(&config.keymap.toggle_mode);
        let switch_chord = parse_key_chord(&config.keymap.switch_workspace);
        let input_decoder = InputDecoder::new(toggle_chord.clone(), switch_chord.clone());
        let repo_status_tx = spawn_repo_status_worker(event_tx.clone());

        // Prune stale bp.* worktrees on startup
        if let Some(root) = repo_root.as_ref() {
            let _ = prune_stale_workspaces(root, &config.workspace.root);
        }

        // Restore previous workspace if available.
        let mut active_workspace = None;
        let mut app_cwd = cwd.clone();

        if let (Some(root), Some(state)) = (repo_root.clone(), load_state()) {
            if let Some(path) = get_active_workspace(&state, &root) {
                // Only restore if the path still exists and matches a current worktree.
                let mut restored = false;
                if path.is_dir() {
                    if let Some(name) =
                        workspace_name_from_path(&root, &config.workspace.root, &path)
                    {
                        let names = list_workspace_names(&root, &config.workspace.root);
                        if names.iter().any(|candidate| candidate == &name) {
                            active_workspace = Some(name);
                            app_cwd = path;
                            repo_root = resolve_repo_root(&app_cwd).or(repo_root);
                            restored = true;
                        }
                    }
                }

                if !restored {
                    // Drop stale state so we don't keep trying a deleted workspace.
                    let _ = remove_active_workspace(&root);
                }
            }
        }

        let mode = if active_workspace.is_some() {
            Mode::Work
        } else {
            Mode::Manage
        };
        let mut app = Self {
            mode,
            command_active: false,
            command_input: String::new(),
            output: None,
            cwd: app_cwd,
            repo_root,
            active_workspace,
            toggle_chord,
            switch_chord,
            input_decoder,
            should_quit: false,
            config,
            sessions: std::collections::HashMap::new(),
            overlay: WorkspaceOverlay::default(),
            prompt_overlay: PromptOverlay::default(),
            command_overlay: CommandOverlay::default(),
            event_tx,
            repo_status: RepoStatus::default(),
            repo_status_tx: Some(repo_status_tx),
            terminal_seq: 0,
            terminal_area: None,
            loading: None,
            pending_command: None,
            refresh_requested: false,
            input_modes_applied: InputModes::default(),
            pending_input_mode_bytes: Vec::new(),
            pre_overlay_mode: None,
        };

        if let Err(err) = super::input::ensure_active_workspace_session(&mut app, 24, 80) {
            app.set_output(err);
        }

        if let Some(tx) = app.repo_status_tx.as_ref() {
            let _ = tx.send(RepoStatusSignal::Request(app.cwd.clone()));
        }

        app.sync_input_modes_for_mode();
        app
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.sync_input_modes_for_mode();
    }

    pub(crate) fn queue_input_mode_target(&mut self, target: InputModes) {
        if target == self.input_modes_applied {
            return;
        }
        let diff = target.diff_bytes(&self.input_modes_applied);
        if !diff.is_empty() {
            self.pending_input_mode_bytes.extend_from_slice(&diff);
        }
        self.input_modes_applied = target;
    }

    fn sync_input_modes_for_mode(&mut self) {
        match self.mode {
            Mode::Manage => self.queue_input_mode_target(InputModes::default()),
            Mode::Work => {
                let Some(active) = self.active_workspace.as_deref() else {
                    return;
                };
                let Some(session) = self.sessions.get(active) else {
                    return;
                };
                self.queue_input_mode_target(session.terminal.input_modes());
            }
        }
    }

    /// Set the output message shown in the command bar area.
    pub fn set_output(&mut self, message: String) {
        let trimmed = message.trim().to_string();
        if trimmed.is_empty() {
            self.output = None;
        } else {
            self.output = Some(trimmed);
        }
    }
}

fn flush_pending_input_modes(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    if app.pending_input_mode_bytes.is_empty() {
        return Ok(());
    }
    terminal
        .backend_mut()
        .write_all(&app.pending_input_mode_bytes)?;
    terminal.backend_mut().flush()?;
    app.pending_input_mode_bytes.clear();
    Ok(())
}

fn sync_resize(sender: &Sender<AppEvent>, last_size: &mut Option<(u16, u16)>) -> bool {
    let size = crossterm::terminal::size()
        .ok()
        .map(|(cols, rows)| (rows, cols));
    if let Some(size) = size {
        let changed = *last_size != Some(size);
        if changed {
            *last_size = Some(size);
            return sender.send(AppEvent::Resize).is_ok();
        }
    }
    true
}

fn read_raw_bytes(timeout: Duration) -> io::Result<Option<Vec<u8>>> {
    #[cfg(unix)]
    {
        use std::io::Read;
        use std::os::unix::io::AsRawFd;

        let fd = io::stdin().as_raw_fd();
        let mut fds = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout_ms = timeout.as_millis().try_into().unwrap_or(i32::MAX);
        let rc = unsafe { libc::poll(&mut fds, 1, timeout_ms) };
        if rc == 0 {
            return Ok(None);
        }
        if rc < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                return Ok(None);
            }
            return Err(err);
        }
        if (fds.revents & libc::POLLIN) == 0 {
            return Ok(None);
        }
        let mut buffer = [0u8; 4096];
        let size = io::stdin().read(&mut buffer)?;
        if size == 0 {
            return Ok(None);
        }
        Ok(Some(buffer[..size].to_vec()))
    }
    #[cfg(not(unix))]
    {
        use std::io::Read;

        if !event::poll(timeout)? {
            return Ok(None);
        }
        let mut buffer = [0u8; 4096];
        let size = io::stdin().read(&mut buffer)?;
        if size == 0 {
            return Ok(None);
        }
        Ok(Some(buffer[..size].to_vec()))
    }
}
