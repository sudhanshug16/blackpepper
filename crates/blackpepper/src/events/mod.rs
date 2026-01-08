//! Application event types.
//!
//! Defines the event enum used for communication between the input
//! thread (key events or raw bytes), PTY reader threads, and the main event loop.
//!
//! Events are sent via mpsc channels and processed sequentially
//! in the main loop to update app state and trigger re-renders.

use std::path::PathBuf;

use crossterm::event::KeyEvent;

use crate::repo_status::RepoStatus;

#[derive(Debug)]
pub enum AppEvent {
    Input(KeyEvent),
    RawInput(Vec<u8>),
    PtyOutput(u64, Vec<u8>),
    PtyExit(u64),
    Resize(u16, u16),
    CommandOutput {
        name: String,
        chunk: String,
    },
    CommandPhaseComplete {
        phase: crate::commands::CommandPhase,
    },
    CommandDone {
        name: String,
        args: Vec<String>,
        result: crate::commands::CommandResult,
    },
    RepoStatusUpdated {
        cwd: PathBuf,
        status: RepoStatus,
    },
}
