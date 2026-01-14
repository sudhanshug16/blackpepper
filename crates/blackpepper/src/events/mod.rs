//! Application event types.
//!
//! Defines the event enum used for communication between the input
//! thread (key events or raw bytes), PTY reader threads, and the main event loop.
//!
//! Events are sent via mpsc channels and processed sequentially
//! in the main loop to update app state and trigger re-renders.

use std::path::PathBuf;

use crate::repo_status::RepoStatus;

#[derive(Debug)]
pub enum AppEvent {
    RawInput(Vec<u8>),
    InputFlush,
    PtyOutput(u64, Vec<u8>),
    PtyExit(u64),
    Resize,
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
