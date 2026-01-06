//! Application state types and core data structures.
//!
//! Defines the App struct which holds all mutable application state,
//! plus supporting types for modes, overlays, tabs, and selection.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use crossterm::event::MouseButton;
use ratatui::layout::Rect;

use crate::config::Config;
use crate::events::AppEvent;
use crate::keymap::KeyChord;
use crate::terminal::TerminalSession;

/// UI mode determines which keys are intercepted vs passed to terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Keys go to the terminal (except toggle chord).
    Work,
    /// Keys are handled by the app for navigation/commands.
    Manage,
}

/// Workspace selection overlay state.
#[derive(Debug, Default)]
pub struct WorkspaceOverlay {
    pub visible: bool,
    pub items: Vec<String>,
    pub message: Option<String>,
    pub selected: usize,
}

/// Tab selection overlay state.
#[derive(Debug, Default)]
pub struct TabOverlay {
    pub visible: bool,
    pub items: Vec<usize>,
    pub selected: usize,
}

/// Terminal cell position (row, col).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CellPos {
    pub row: u16,
    pub col: u16,
}

/// Text selection state for copy/paste.
#[derive(Debug, Default)]
pub struct SelectionState {
    pub active: bool,
    pub selecting: bool,
    pub start: Option<CellPos>,
    pub end: Option<CellPos>,
}

/// Search match location within visible terminal.
#[derive(Debug, Clone, Copy)]
pub struct SearchMatch {
    pub row: u16,
    pub start: u16,
    pub end: u16,
}

/// In-terminal search state.
#[derive(Debug, Default)]
pub struct SearchState {
    pub active: bool,
    pub query: String,
    pub matches: Vec<SearchMatch>,
    pub active_index: usize,
}

/// A single tab within a workspace.
pub struct WorkspaceTab {
    pub id: u64,
    pub name: String,
    pub explicit_name: bool,
    pub terminal: TerminalSession,
}

/// Collection of tabs for a workspace.
pub struct WorkspaceTabs {
    pub tabs: Vec<WorkspaceTab>,
    pub active: usize,
    pub next_index: usize,
}

impl WorkspaceTabs {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            next_index: 1,
        }
    }
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
    pub switch_tab_chord: Option<KeyChord>,
    pub refresh_chord: Option<KeyChord>,
    pub should_quit: bool,
    pub config: Config,
    pub tabs: HashMap<String, WorkspaceTabs>,
    pub overlay: WorkspaceOverlay,
    pub tab_overlay: TabOverlay,
    pub event_tx: Sender<AppEvent>,
    pub terminal_seq: u64,
    pub tab_bar_area: Option<Rect>,
    pub terminal_area: Option<Rect>,
    pub mouse_debug: bool,
    pub mouse_pressed: Option<MouseButton>,
    pub mouse_log_path: Option<PathBuf>,
    pub loading: Option<String>,
    pub selection: SelectionState,
    pub search: SearchState,
    pub refresh_requested: bool,
}

pub const MAX_TAB_LABEL_LEN: usize = 20;
pub const OUTPUT_MAX_LINES: usize = 6;
pub const BOTTOM_HORIZONTAL_PADDING: u16 = 1;
pub const SCROLL_LINES: isize = 3;
