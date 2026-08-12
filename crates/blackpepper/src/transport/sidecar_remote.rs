use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::sidecar::SidecarError;
use super::sidecar_cache::CachedSidecar;
use super::{CommandOutput, HostTransport, TransportError};

const SIDECAR_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

mod upload;
pub(crate) use upload::upload_file_to_child;

/// Installed remote executable with the exact release and content identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSidecar {
    pub binary_path: PathBuf,
    pub binary_sha256: String,
}

#[derive(Debug)]
pub enum SidecarInstallError {
    Sidecar(SidecarError),
    Transport(TransportError),
    Io {
        operation: String,
        source: io::Error,
    },
    RemoteCommand {
        operation: &'static str,
        status: Option<i32>,
        stderr: String,
    },
    MissingUploadStdin,
    UploadStalled {
        transferred: u64,
        total: u64,
        cancellation_error: Option<String>,
    },
    UploadTimedOut {
        transferred: u64,
        total: u64,
        cancellation_error: Option<String>,
    },
    UploadCancelled {
        transferred: u64,
        total: u64,
        cancellation_error: Option<String>,
    },
}

impl fmt::Display for SidecarInstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sidecar(error) => error.fmt(formatter),
            Self::Transport(error) => error.fmt(formatter),
            Self::Io { operation, source } => write!(formatter, "{operation}: {source}"),
            Self::RemoteCommand {
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
            Self::MissingUploadStdin => {
                formatter.write_str("sidecar upload channel did not provide writable stdin")
            }
            Self::UploadStalled {
                transferred,
                total,
                cancellation_error,
            } => {
                write!(
                    formatter,
                    "managed sidecar upload stopped making progress after {transferred}/{total} bytes"
                )?;
                write_cancellation_error(formatter, cancellation_error)
            }
            Self::UploadTimedOut {
                transferred,
                total,
                cancellation_error,
            } => {
                write!(
                    formatter,
                    "managed sidecar upload exceeded its 120-second deadline after {transferred}/{total} bytes"
                )?;
                write_cancellation_error(formatter, cancellation_error)
            }
            Self::UploadCancelled {
                transferred,
                total,
                cancellation_error,
            } => {
                write!(
                    formatter,
                    "managed sidecar upload was cancelled after {transferred}/{total} bytes"
                )?;
                write_cancellation_error(formatter, cancellation_error)
            }
        }
    }
}

impl std::error::Error for SidecarInstallError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sidecar(error) => Some(error),
            Self::Transport(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<SidecarError> for SidecarInstallError {
    fn from(error: SidecarError) -> Self {
        Self::Sidecar(error)
    }
}

impl From<TransportError> for SidecarInstallError {
    fn from(error: TransportError) -> Self {
        Self::Transport(error)
    }
}

/// Upload and atomically install a cached sidecar on a Linux workspace host.
pub fn install_remote(
    transport: &mut dyn HostTransport,
    cached: &CachedSidecar,
    remote_home: &Path,
) -> Result<RemoteSidecar, SidecarInstallError> {
    let plan = cached.upload_plan(remote_home)?;
    install_plan(transport, plan)
}

/// Install using the host's explicit absolute `XDG_DATA_HOME`.
pub fn install_remote_in_data_home(
    transport: &mut dyn HostTransport,
    cached: &CachedSidecar,
    remote_data_home: &Path,
) -> Result<RemoteSidecar, SidecarInstallError> {
    let plan = cached.upload_plan_in_data_home(remote_data_home)?;
    install_plan(transport, plan)
}

fn install_plan(
    transport: &mut dyn HostTransport,
    plan: super::sidecar_upload::UploadPlan,
) -> Result<RemoteSidecar, SidecarInstallError> {
    require_success(
        "preparing remote sidecar directory",
        transport.exec_timeout(&plan.prepare_command(), SIDECAR_COMMAND_TIMEOUT)?,
    )?;

    let result = (|| {
        let child = transport.spawn_exec_with_stdin(&plan.receive_command())?;
        require_success(
            "uploading remote sidecar",
            upload_file_to_child(child, &plan.local_binary)?,
        )?;
        require_success(
            "verifying remote sidecar",
            transport.exec_timeout(&plan.verify_and_commit_command(), SIDECAR_COMMAND_TIMEOUT)?,
        )?;
        Ok(RemoteSidecar {
            binary_path: plan.remote_binary.clone(),
            binary_sha256: plan.binary_sha256.clone(),
        })
    })();

    if result.is_err() {
        // Cleanup is best-effort and must never replace the primary failure.
        // Mask the cancelled restore scope just for this bounded, idempotent
        // removal so a cancelled upload cannot strand its unique temp file.
        super::CommandCancellation::mask_current(|| {
            let _ = transport.exec_timeout(&plan.cleanup_command(), CLEANUP_TIMEOUT);
        });
    }
    result
}

fn write_cancellation_error(
    formatter: &mut fmt::Formatter<'_>,
    error: &Option<String>,
) -> fmt::Result {
    if let Some(error) = error {
        write!(formatter, "; cancellation also failed: {error}")
    } else {
        Ok(())
    }
}

fn require_success(
    operation: &'static str,
    output: CommandOutput,
) -> Result<(), SidecarInstallError> {
    if output.success {
        return Ok(());
    }
    Err(SidecarInstallError::RemoteCommand {
        operation,
        status: output.status,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_deadline_errors_report_exact_progress_without_path_content() {
        let error = SidecarInstallError::UploadStalled {
            transferred: 12,
            total: 34,
            cancellation_error: None,
        };

        assert_eq!(
            error.to_string(),
            "managed sidecar upload stopped making progress after 12/34 bytes"
        );
        assert!(upload::UPLOAD_STALL_TIMEOUT < upload::UPLOAD_TOTAL_TIMEOUT);
    }
}
