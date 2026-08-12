//! Execution primitives shared by local and SSH-backed workspaces.
//!
//! The transport boundary deliberately deals in process specifications instead
//! of terminal screen state. Callers can therefore feed [`PtyProcess`] into the
//! existing terminal renderer without teaching that renderer about SSH.

mod cancellation;
mod command;
mod local;
mod process_cancel;
#[cfg(all(test, unix))]
mod process_cancel_tests;
mod pty;
mod sidecar;
mod sidecar_archive;
mod sidecar_cache;
mod sidecar_cache_fs;
#[cfg(test)]
mod sidecar_cache_tests;
mod sidecar_download;
mod sidecar_manifest;
mod sidecar_remote;
mod sidecar_upload;
mod ssh;
mod ssh_cancel;
#[cfg(test)]
mod ssh_cancel_tests;
mod ssh_command;
mod ssh_config;
#[cfg(test)]
mod ssh_tests;

pub(crate) use cancellation::CommandCancellation;
pub use command::{CommandOutput, HostCommand, ProcessSpec, RunningCommand};
pub use local::LocalTransport;
pub use pty::{PtyExit, PtyProcess};
pub use sidecar::{
    release_asset, select_runtime, sha256_bytes, sha256_file, ArchiveKind, ManagedTool,
    ReleaseAsset, RuntimeSelection, SidecarError, SidecarTarget, SystemRuntime, VerifiedArchive,
    WORKTRUNK_VERSION, ZELLIJ_VERSION,
};
pub use sidecar_cache::{CachedSidecar, SidecarCache};
pub use sidecar_download::{HttpDownloader, SidecarDownloader};
pub(crate) use sidecar_remote::upload_file_to_child;
pub use sidecar_remote::{
    install_remote, install_remote_in_data_home, RemoteSidecar, SidecarInstallError,
};
pub use sidecar_upload::UploadPlan;
pub use ssh::{ConnectionState, SshTransport};
pub use ssh_config::{ControlSocket, SshConfig, CONTROL_PATH_MAX_BYTES};

use std::fmt;
use std::io;
use std::net::IpAddr;
use std::time::Duration;

use portable_pty::PtySize;

/// Whether commands run directly on this machine or through OpenSSH.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    Local,
    Ssh,
}

/// A local listener backed by a port on the workspace host.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalForward {
    pub bind_address: IpAddr,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
}

impl LocalForward {
    pub fn loopback(local_port: u16, remote_port: u16) -> Self {
        Self {
            bind_address: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            local_port,
            remote_host: "127.0.0.1".to_string(),
            remote_port,
        }
    }

    pub fn validate(&self) -> Result<(), TransportError> {
        if self.local_port == 0 || self.remote_port == 0 {
            return Err(TransportError::InvalidForward(
                "local and remote ports must be non-zero".to_string(),
            ));
        }
        if self.remote_host.trim().is_empty() || self.remote_host.chars().any(char::is_whitespace) {
            return Err(TransportError::InvalidForward(
                "remote host must be a non-empty host name or address".to_string(),
            ));
        }
        Ok(())
    }
}

/// Common execution surface used by the workspace runtime.
pub trait HostTransport {
    fn kind(&self) -> HostKind;

    fn spawn_exec(&mut self, command: &HostCommand) -> Result<RunningCommand, TransportError>;

    /// Spawn an exec channel with writable stdin for uploads or helper IPC.
    /// Ordinary commands keep stdin closed through [`HostTransport::spawn_exec`].
    fn spawn_exec_with_stdin(
        &mut self,
        command: &HostCommand,
    ) -> Result<RunningCommand, TransportError>;

    fn attach_pty(
        &mut self,
        command: &HostCommand,
        size: PtySize,
    ) -> Result<PtyProcess, TransportError>;

    fn forward_local_port(&mut self, forward: LocalForward)
        -> Result<LocalForward, TransportError>;

    fn cancel_local_forward(&mut self, forward: &LocalForward) -> Result<(), TransportError>;

    fn exec(&mut self, command: &HostCommand) -> Result<CommandOutput, TransportError> {
        self.spawn_exec(command)?.wait_with_output()
    }

    /// Execute bounded-output metadata work with an explicit deadline.
    ///
    /// This is deliberately opt-in: mutations and streaming commands need
    /// operation-specific disconnect reconciliation instead of a generic
    /// timeout that could hide an unknown remote result.
    fn exec_timeout(
        &mut self,
        command: &HostCommand,
        timeout: Duration,
    ) -> Result<CommandOutput, TransportError> {
        self.spawn_exec(command)?.wait_with_output_timeout(timeout)
    }
}

#[derive(Debug)]
pub enum TransportError {
    InvalidCommand(String),
    InvalidEnvironment(String),
    InvalidForward(String),
    InvalidSshConfiguration(String),
    ControlPathTooLong {
        path: String,
        max_bytes: usize,
    },
    AlreadyConnected,
    NotConnected,
    MasterExited(Option<u32>),
    CancellationTimedOut {
        process_id: u32,
    },
    CommandTimedOut {
        process_id: u32,
        timeout_ms: u64,
        cancellation_error: Option<String>,
    },
    CommandCancelled {
        process_id: u32,
        cancellation_error: Option<String>,
    },
    ForwardNotOwned(LocalForward),
    CommandFailed {
        operation: String,
        status: Option<i32>,
        stderr: String,
    },
    Io {
        operation: String,
        source: io::Error,
    },
    Pty(String),
    Unsupported(&'static str),
}

impl TransportError {
    pub(crate) fn io(operation: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            operation: operation.into(),
            source,
        }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCommand(message)
            | Self::InvalidEnvironment(message)
            | Self::InvalidForward(message)
            | Self::InvalidSshConfiguration(message)
            | Self::Pty(message) => formatter.write_str(message),
            Self::ControlPathTooLong { path, max_bytes } => write!(
                formatter,
                "SSH control path is too long ({path}); it must be under {max_bytes} bytes"
            ),
            Self::AlreadyConnected => formatter.write_str("SSH transport is already connected"),
            Self::NotConnected => formatter.write_str("SSH transport is not connected"),
            Self::MasterExited(status) => {
                write!(formatter, "SSH control master exited")?;
                if let Some(status) = status {
                    write!(formatter, " with status {status}")?;
                }
                Ok(())
            }
            Self::CancellationTimedOut { process_id } => write!(
                formatter,
                "process {process_id} did not exit before the cancellation deadline"
            ),
            Self::CommandTimedOut {
                process_id,
                timeout_ms,
                cancellation_error,
            } => {
                write!(
                    formatter,
                    "command process {process_id} exceeded its {timeout_ms}ms deadline"
                )?;
                if let Some(error) = cancellation_error {
                    write!(formatter, "; cancellation also failed: {error}")?;
                }
                Ok(())
            }
            Self::CommandCancelled {
                process_id,
                cancellation_error,
            } => {
                write!(formatter, "command process {process_id} was cancelled")?;
                if let Some(error) = cancellation_error {
                    write!(formatter, "; cancellation also failed: {error}")?;
                }
                Ok(())
            }
            Self::ForwardNotOwned(forward) => write!(
                formatter,
                "Blackpepper did not create the local forward on port {}",
                forward.local_port
            ),
            Self::CommandFailed {
                operation,
                status,
                stderr,
            } => {
                write!(formatter, "{operation} failed")?;
                if let Some(status) = status {
                    write!(formatter, " with status {status}")?;
                }
                if !stderr.trim().is_empty() {
                    write!(formatter, ": {}", stderr.trim())?;
                }
                Ok(())
            }
            Self::Io { operation, source } => write!(formatter, "{operation}: {source}"),
            Self::Unsupported(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for TransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
