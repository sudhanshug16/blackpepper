//! Application runner and event loop.
//!
//! Handles terminal setup/teardown and the main event loop.
//! Events are read from an mpsc channel and dispatched to handlers.

use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use crossterm::event::{
    self, Event, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::config::load_config;
use crate::events::AppEvent;
use crate::git::resolve_repo_root;
use crate::keymap::{parse_control_byte, parse_key_chord, DEFAULT_WORK_TOGGLE_BYTE};
use crate::repo_status::{spawn_repo_status_worker, RepoStatus, RepoStatusSignal};
use crate::state::{get_active_workspace, load_state, remove_active_workspace};
use crate::workspaces::{list_workspace_names, workspace_name_from_path};

use super::state::{App, CommandOverlay, InputModeHandle, Mode, PromptOverlay, WorkspaceOverlay};

/// Entry point: set up terminal and run the event loop.
pub fn run() -> io::Result<()> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(PushKeyboardEnhancementFlags(
        KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS,
    ))?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal);

    disable_raw_mode()?;
    terminal
        .backend_mut()
        .execute(PopKeyboardEnhancementFlags)?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// Main event loop: process events until quit.
fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let (event_tx, event_rx) = mpsc::channel::<AppEvent>();
    let mut app = App::new(event_tx.clone());
    spawn_input_thread(event_tx.clone(), app.input_mode.clone());
    terminal.clear()?;
    terminal.draw(|frame| super::render::render(&mut app, frame))?;

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

        terminal.draw(|frame| super::render::render(&mut app, frame))?;
    }
    Ok(())
}

/// Spawn a thread to read terminal input (key events in manage mode, raw bytes in work mode).
fn spawn_input_thread(sender: Sender<AppEvent>, input_mode: InputModeHandle) {
    std::thread::spawn(move || {
        let mut last_size: Option<(u16, u16)> = None;
        loop {
            let mode = input_mode.get();
            let mut sent = false;
            match mode {
                Mode::Manage => {
                    if event::poll(Duration::from_millis(25)).unwrap_or(false) {
                        match event::read() {
                            Ok(Event::Key(key)) => {
                                sent = sender.send(AppEvent::Input(key)).is_ok();
                            }
                            Ok(Event::Resize(cols, rows)) => {
                                last_size = Some((rows, cols));
                                sent = sender.send(AppEvent::Resize).is_ok();
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                }
                Mode::Work => {
                    match read_raw_bytes(Duration::from_millis(25)) {
                        Ok(Some(bytes)) => {
                            sent = sender.send(AppEvent::RawInput(bytes)).is_ok();
                        }
                        Ok(None) => {}
                        Err(_) => break,
                    }
                }
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
        let work_toggle_byte =
            parse_control_byte(&config.keymap.toggle_mode).unwrap_or(DEFAULT_WORK_TOGGLE_BYTE);
        let switch_chord = parse_key_chord(&config.keymap.switch_workspace);
        let switch_tab_chord = parse_key_chord(&config.keymap.switch_tab);
        let refresh_chord = parse_key_chord(&config.keymap.refresh);
        let repo_status_tx = spawn_repo_status_worker(event_tx.clone());

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
        let input_mode = InputModeHandle::new(mode);
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
            switch_tab_chord,
            refresh_chord,
            work_toggle_byte,
            input_mode,
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
        };

        if let Err(err) = super::input::ensure_active_workspace_session(&mut app, 24, 80) {
            app.set_output(err);
        }

        if let Some(tx) = app.repo_status_tx.as_ref() {
            let _ = tx.send(RepoStatusSignal::Request(app.cwd.clone()));
        }

        app
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.input_mode.set(mode);
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

fn sync_resize(sender: &Sender<AppEvent>, last_size: &mut Option<(u16, u16)>) -> bool {
    let size = crossterm::terminal::size().ok().map(|(cols, rows)| (rows, cols));
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
        let timeout_ms = timeout
            .as_millis()
            .try_into()
            .unwrap_or(i32::MAX);
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
        return Ok(Some(buffer[..size].to_vec()));
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
