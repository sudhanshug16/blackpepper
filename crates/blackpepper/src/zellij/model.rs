use std::fmt;

use serde::Deserialize;

use crate::transport::{CommandOutput, TransportError};

mod missing_session;

pub(crate) use missing_session::{client_list_reports_missing_session, reports_no_active_session};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientOperation {
    /// Starting another client never changes an existing client's focus.
    Attach,
    /// Zellij 0.44.3 still focuses a dynamically created `focus=false` tab.
    /// Blackpepper can restore one known client's tab, but cannot safely pick
    /// among multiple independently focused clients.
    BackgroundMutation,
    /// An out-of-band action resolved against Zellij's last-active client.
    FocusChange,
    /// Session destruction is only safe when nobody is attached.
    Destroy,
}

impl ClientOperation {
    pub(crate) fn allows(self, client_count: usize) -> bool {
        match self {
            Self::Attach => true,
            Self::BackgroundMutation | Self::FocusChange => client_count <= 1,
            Self::Destroy => client_count == 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZellijClient {
    pub client_id: u32,
    pub pane_id: String,
    pub running_command: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ZellijTab {
    pub tab_id: u64,
    pub position: usize,
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ZellijPane {
    pub id: u32,
    pub is_plugin: bool,
    pub tab_id: u64,
    /// Name of the owning tab, used with `tab_id` to reject ID reuse.
    pub tab_name: String,
    /// Whether the pane's command has exited while the pane remains visible.
    pub exited: bool,
    /// The command's status when Zellij was able to observe a normal exit.
    #[serde(deserialize_with = "deserialize_required_option")]
    pub exit_status: Option<i32>,
    /// A command pane waiting for the user to rerun or close it is not live.
    pub is_held: bool,
    /// The command configured for a command pane, including its arguments.
    #[serde(deserialize_with = "deserialize_required_option")]
    pub terminal_command: Option<String>,
    /// Zellij's best-effort description of the process currently in the pane.
    #[serde(default)]
    pub pane_command: Option<String>,
}

/// Process state observable through Zellij's pinned `list-panes` JSON API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneProcessState {
    Live,
    Exited {
        code: Option<i32>,
    },
    Missing,
    /// The numeric selector exists, but its immutable launch marker does not.
    /// It may be a pane ID reused after a session restart, so neither liveness
    /// nor exit can be inferred safely.
    UnverifiedIdentity {
        location_changed: bool,
    },
}

impl ZellijPane {
    pub fn selector(&self) -> String {
        format!(
            "{}_{}",
            if self.is_plugin { "plugin" } else { "terminal" },
            self.id
        )
    }

    pub fn process_state(&self) -> PaneProcessState {
        if self.exited || self.is_held {
            PaneProcessState::Exited {
                code: self.exit_status,
            }
        } else {
            PaneProcessState::Live
        }
    }

    /// Zellij reports the original command and arguments for command panes.
    /// Blackpepper embeds one UUID-valued environment argument at launch; it
    /// survives native tab rename/move operations and distinguishes pane-ID
    /// reuse without inspecting terminal contents or the live process tree.
    pub fn has_command_argument(&self, expected: &str) -> bool {
        self.terminal_command
            .as_deref()
            .is_some_and(|command| command.split_whitespace().any(|part| part == expected))
    }
}

pub(crate) fn classify_pane_process(
    panes: &[ZellijPane],
    tab_id: u64,
    tab_name: &str,
    pane_selector: &str,
    expected_command_argument: &str,
) -> PaneProcessState {
    let Some(pane) = panes.iter().find(|pane| pane.selector() == pane_selector) else {
        return PaneProcessState::Missing;
    };
    if pane.has_command_argument(expected_command_argument) {
        return pane.process_state();
    }
    PaneProcessState::UnverifiedIdentity {
        location_changed: pane.tab_id != tab_id || pane.tab_name != tab_name,
    }
}

/// `Option` normally treats a missing Serde field like JSON `null`. The
/// lifecycle observer must distinguish those cases so a changed Zellij schema
/// cannot silently turn an exited pane into a live pane.
fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

pub(crate) fn parse_clients(value: &str) -> Result<Vec<ZellijClient>, ZellijError> {
    let mut lines = value.lines().filter(|line| !line.trim().is_empty());
    if lines.next().map(str::trim) != Some("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND") {
        return Err(ZellijError::InvalidOutput(
            "unexpected Zellij list-clients header".to_string(),
        ));
    }
    lines
        .map(|line| {
            let mut fields = line.split_whitespace();
            let client_id = fields
                .next()
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| ZellijError::InvalidOutput(format!("invalid client row: {line}")))?;
            let pane_id = fields
                .next()
                .ok_or_else(|| ZellijError::InvalidOutput(format!("invalid client row: {line}")))?;
            Ok(ZellijClient {
                client_id,
                pane_id: pane_id.to_string(),
                running_command: fields.collect::<Vec<_>>().join(" "),
            })
        })
        .collect()
}

pub(crate) fn parse_sessions(output: CommandOutput) -> Result<Vec<String>, ZellijError> {
    if !output.success {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.trim() == "No active zellij sessions found." && output.stdout.is_empty() {
            return Ok(Vec::new());
        }
        return Err(command_error("list Zellij sessions", output));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

pub(crate) fn parse_panes(value: &[u8]) -> Result<Vec<ZellijPane>, ZellijError> {
    serde_json::from_slice(value)
        .map_err(|error| ZellijError::InvalidOutput(format!("invalid pane JSON: {error}")))
}

pub(crate) fn checked(
    output: CommandOutput,
    operation: &str,
) -> Result<CommandOutput, ZellijError> {
    if output.success {
        Ok(output)
    } else {
        Err(command_error(operation, output))
    }
}

pub(crate) fn command_error(operation: &str, output: CommandOutput) -> ZellijError {
    ZellijError::CommandFailed {
        operation: operation.to_string(),
        status: output.status,
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    }
}

#[derive(Debug)]
pub enum ZellijError {
    Transport(TransportError),
    InvalidName(String),
    InvalidOutput(String),
    VersionMismatch {
        expected: String,
        actual: String,
    },
    ClientConflict {
        operation: ClientOperation,
        clients: Vec<ZellijClient>,
    },
    AmbiguousSessionNamespace {
        session: String,
        socket_directories: Vec<String>,
    },
    /// The exact pre-PTY `list-clients` query observed that its session
    /// disappeared. Callers may recreate and retry once because no attached
    /// PTY has been spawned yet.
    SessionMissingBeforeAttach,
    CommandFailed {
        operation: String,
        status: Option<i32>,
        stderr: String,
    },
}

impl From<TransportError> for ZellijError {
    fn from(error: TransportError) -> Self {
        Self::Transport(error)
    }
}

impl fmt::Display for ZellijError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => error.fmt(formatter),
            Self::InvalidName(message) | Self::InvalidOutput(message) => {
                formatter.write_str(message)
            }
            Self::VersionMismatch { expected, actual } => {
                write!(formatter, "Zellij {expected} is required; found {actual}")
            }
            Self::ClientConflict { operation, clients } => write!(
                formatter,
                "refusing {operation:?}: Zellij session has {} controlling client(s)",
                clients.len()
            ),
            Self::AmbiguousSessionNamespace {
                session,
                socket_directories,
            } => write!(
                formatter,
                "Zellij session {session:?} is live in multiple socket namespaces ({}); refusing to choose one",
                socket_directories.join(", ")
            ),
            Self::SessionMissingBeforeAttach => formatter
                .write_str("the Zellij session disappeared before its client PTY was attached"),
            Self::CommandFailed {
                operation,
                status,
                stderr,
            } => write!(
                formatter,
                "{operation} failed (status {status:?}): {stderr}"
            ),
        }
    }
}

impl std::error::Error for ZellijError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            _ => None,
        }
    }
}
