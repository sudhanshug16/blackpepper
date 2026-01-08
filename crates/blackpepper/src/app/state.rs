//! Application state types and core data structures.
//!
//! Defines the App struct which holds all mutable application state,
//! plus supporting types for modes, overlays, and workspaces.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use ratatui::layout::Rect;

use crate::config::Config;
use crate::events::AppEvent;
use crate::keymap::KeyChord;
use crate::repo_status::{RepoStatus, RepoStatusSignal};
use crate::terminal::TerminalSession;

/// UI mode determines which keys are intercepted vs passed to terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Raw input goes to the terminal (except the toggle control byte).
    Work,
    /// Keys are handled by the app for navigation/commands.
    Manage,
}

#[derive(Clone)]
pub struct InputModeHandle {
    mode: Arc<AtomicU8>,
}

impl InputModeHandle {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode: Arc::new(AtomicU8::new(mode_to_u8(mode))),
        }
    }

    pub fn get(&self) -> Mode {
        match self.mode.load(Ordering::Relaxed) {
            1 => Mode::Work,
            _ => Mode::Manage,
        }
    }

    pub fn set(&self, mode: Mode) {
        self.mode.store(mode_to_u8(mode), Ordering::Relaxed);
    }
}

fn mode_to_u8(mode: Mode) -> u8 {
    match mode {
        Mode::Manage => 0,
        Mode::Work => 1,
    }
}

/// Workspace selection overlay state.
#[derive(Debug, Default)]
pub struct WorkspaceOverlay {
    pub visible: bool,
    pub items: Vec<String>,
    pub message: Option<String>,
    pub selected: usize,
}

/// Generic selection overlay (e.g., agent provider selection).
#[derive(Debug, Default)]
pub struct PromptOverlay {
    pub visible: bool,
    pub items: Vec<String>,
    pub message: Option<String>,
    pub selected: usize,
    pub title: String,
}

/// Streaming command output overlay (e.g., PR generation).
#[derive(Debug, Default)]
pub struct CommandOverlay {
    pub visible: bool,
    pub title: String,
    pub output: String,
}

#[derive(Debug, Clone)]
pub struct PendingCommand {
    pub name: String,
    pub args: Vec<String>,
}

/// A single tmux-backed terminal session per workspace.
pub struct WorkspaceSession {
    pub terminal: TerminalSession,
}

/// Main application state container.
///
/// Holds all mutable state for the TUI application. Methods are
/// split across input.rs (event handling) and render.rs (UI drawing).
pub struct App {
    pub mode: Mode,
    pub command_active: bool,
    pub command_input: String,
    pub output: Option<String>,
    pub cwd: PathBuf,
    pub repo_root: Option<PathBuf>,
    pub active_workspace: Option<String>,
    pub toggle_chord: Option<KeyChord>,
    pub switch_chord: Option<KeyChord>,
    pub refresh_chord: Option<KeyChord>,
    pub work_toggle_byte: u8,
    pub input_mode: InputModeHandle,
    pub should_quit: bool,
    pub config: Config,
    pub sessions: HashMap<String, WorkspaceSession>,
    pub overlay: WorkspaceOverlay,
    pub prompt_overlay: PromptOverlay,
    pub command_overlay: CommandOverlay,
    pub event_tx: Sender<AppEvent>,
    pub repo_status: RepoStatus,
    pub repo_status_tx: Option<Sender<RepoStatusSignal>>,
    pub terminal_seq: u64,
    pub terminal_area: Option<Rect>,
    pub loading: Option<String>,
    pub pending_command: Option<PendingCommand>,
    pub refresh_requested: bool,
}

pub const OUTPUT_MAX_LINES: usize = 6;
pub const BOTTOM_HORIZONTAL_PADDING: u16 = 1;
