//! Remote-first interactive client.

mod actions;
mod command;
mod control;
mod model;
mod mouse;
mod render;
mod runner;
mod runtime;
mod state;
mod terminal;

pub use command::{parse as parse_command, ClientCommand, HELP as COMMAND_HELP};
pub use model::{
    build_tree, DisplayStatus, HostConnection, HostNode, RepositoryNode, WorkspaceNode,
};
pub use render::render;
pub use runner::run;
pub use state::{ClientMode, ClientState};
pub use terminal::EmbeddedTerminal;

use crate::core::WorkspaceId;

#[derive(Debug)]
pub enum ClientEvent {
    RawInput(Vec<u8>),
    InputFlush,
    TerminalOutput(WorkspaceId, uuid::Uuid, Vec<u8>),
    TerminalNotice(WorkspaceId, uuid::Uuid, String),
    TerminalExited(WorkspaceId, uuid::Uuid),
    HostAuthenticationOutput(crate::core::HostId, Vec<u8>),
    BlockerTransition(uuid::Uuid, crate::status_monitor::BlockerTransition),
    BlockerWatcherExited(crate::core::AgentRunId, uuid::Uuid),
    Resize,
    PeriodicRefreshComplete {
        token: uuid::Uuid,
        host_id: crate::core::HostId,
        result: Result<crate::core::HostPeriodicRefresh, String>,
    },
    PeriodicForwardCleanupComplete {
        token: uuid::Uuid,
        host_id: crate::core::HostId,
        outcomes: Vec<runtime::ForwardCleanupOutcome>,
    },
    ConnectionRestoreProgress {
        token: uuid::Uuid,
        host_id: crate::core::HostId,
        message: String,
    },
    ConnectionRestoreComplete {
        token: uuid::Uuid,
        host_id: crate::core::HostId,
    },
    HostOperationProgress {
        token: uuid::Uuid,
        host_id: crate::core::HostId,
        message: String,
    },
    HostOperationComplete {
        token: uuid::Uuid,
        host_id: crate::core::HostId,
        generation: u64,
    },
    ManualRefreshRequested,
    BackgroundResult {
        operation: String,
        result: Result<String, String>,
    },
}
