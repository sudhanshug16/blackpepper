use crate::core::{CorePaths, HostRegistry, WorkspaceId};
use fs2::FileExt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub const SESSION_LEASE_READY: &str = "blackpepper-session-lease-v1";
const SESSION_LEASE_TIMEOUT: Duration = Duration::from_secs(15);
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionLeaseArgs {
    pub workspace_id: WorkspaceId,
    pub session_name: String,
}

impl SessionLeaseArgs {
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Option<Self> {
        let mut workspace_id = None;
        let mut session_name = None;
        let mut arguments = arguments.into_iter();
        while let Some(flag) = arguments.next() {
            let value = arguments.next()?;
            match flag.as_str() {
                "--workspace-id" if workspace_id.is_none() => {
                    workspace_id = Some(value.parse().ok()?);
                }
                "--session" if session_name.is_none() => session_name = Some(value),
                _ => return None,
            }
        }
        Some(Self {
            workspace_id: workspace_id?,
            session_name: session_name?,
        })
    }
}

/// Hold one workspace's Zellij initialization lease until the client closes
/// stdin. The ready line is emitted only after the advisory lock is owned.
pub fn hold_session_lease(
    paths: &CorePaths,
    registry: &HostRegistry,
    arguments: &SessionLeaseArgs,
    mut reader: impl Read,
    mut writer: impl Write,
) -> Result<(), String> {
    // Reject obviously stale requests before waiting, then repeat the full
    // validation after the lock is owned. A remover can journal an unknown
    // result while this helper is blocked; acknowledging a pre-lock snapshot
    // would let another client recreate that workspace in the removal gap.
    validate_lease_context(registry, arguments)?;
    let _lease = SessionInitializationLease::acquire(
        paths,
        arguments.workspace_id,
        &arguments.session_name,
        SESSION_LEASE_TIMEOUT,
    )?;
    validate_lease_context(registry, arguments)?;
    writer
        .write_all(format!("{SESSION_LEASE_READY}\n").as_bytes())
        .and_then(|()| writer.flush())
        .map_err(|error| format!("Could not acknowledge the session lease: {error}"))?;

    let mut buffer = [0_u8; 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(format!("Session lease channel failed: {error}")),
        }
    }
}

fn validate_lease_context(
    registry: &HostRegistry,
    arguments: &SessionLeaseArgs,
) -> Result<(), String> {
    let local_host_id = registry
        .local_host_id()
        .map_err(|error| error.to_string())?;
    let workspace = registry
        .workspace(arguments.workspace_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Session lease workspace is not registered.".to_owned())?;
    if workspace.host_id != local_host_id {
        return Err("Session lease workspace belongs to another host.".to_owned());
    }
    if registry
        .worktrunk_removal(arguments.workspace_id)
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err(
            "This workspace has a Worktrunk removal with an unknown result; run :worktree list before starting or attaching its session."
                .to_owned(),
        );
    }
    let expected = format!("bp-{}", arguments.workspace_id);
    if arguments.session_name != expected {
        return Err("Session lease name does not match its workspace UUID.".to_owned());
    }
    Ok(())
}

pub(super) struct SessionInitializationLease {
    file: File,
}

impl SessionInitializationLease {
    pub(super) fn acquire_for_workspace(
        paths: &CorePaths,
        workspace_id: WorkspaceId,
    ) -> Result<Self, String> {
        let session_name = format!("bp-{workspace_id}");
        Self::acquire(paths, workspace_id, &session_name, SESSION_LEASE_TIMEOUT)
    }

    fn acquire(
        paths: &CorePaths,
        workspace_id: WorkspaceId,
        session_name: &str,
        timeout: Duration,
    ) -> Result<Self, String> {
        let path = lease_path(paths, workspace_id, session_name);
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options
            .open(&path)
            .map_err(|error| format!("Could not open session lease {}: {error}", path.display()))?;
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(|error| {
            format!(
                "Could not make session lease private {}: {error}",
                path.display()
            )
        })?;

        let started = Instant::now();
        loop {
            match FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(Self { file }),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if started.elapsed() >= timeout {
                        return Err(format!(
                            "Another Blackpepper client is still changing session {session_name}; retry after it finishes."
                        ));
                    }
                    std::thread::sleep(LOCK_POLL_INTERVAL);
                }
                Err(error) => {
                    return Err(format!(
                        "Could not acquire session lease {}: {error}",
                        path.display()
                    ));
                }
            }
        }
    }
}

impl Drop for SessionInitializationLease {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn lease_path(paths: &CorePaths, workspace_id: WorkspaceId, session_name: &str) -> PathBuf {
    let identity = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        format!("{workspace_id}:{session_name}").as_bytes(),
    );
    paths.session_lock_dir().join(format!("{identity}.lock"))
}

#[cfg(test)]
#[path = "session_lease_tests.rs"]
mod tests;
