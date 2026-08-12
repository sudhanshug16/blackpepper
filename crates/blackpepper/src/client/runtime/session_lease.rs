use super::ClientRuntime;
use crate::core::{HostId, WorkspaceId};
use crate::host_services::SESSION_LEASE_READY;
use crate::transport::{HostCommand, RunningCommand};
use std::io::{BufRead, BufReader, Read};
use std::process::{ChildStdin, ChildStdout};
use std::time::Duration;

const RELEASE_POLL_ATTEMPTS: usize = 50;
const RELEASE_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Client half of the transient host lease. Keeping stdin open keeps the
/// host-local advisory lock alive; dropping the guard releases it on every
/// error path.
pub(super) struct SessionInitializationLease {
    stdin: Option<ChildStdin>,
    child: Option<RunningCommand>,
}

impl SessionInitializationLease {
    pub(super) fn acquire(
        runtime: &mut ClientRuntime,
        host_id: HostId,
        workspace_id: WorkspaceId,
    ) -> Result<Self, String> {
        let helper = runtime.helper_path(host_id)?;
        let session_name = format!("bp-{workspace_id}");
        let command = HostCommand::new(helper).args([
            "session-lease".to_owned(),
            "--workspace-id".to_owned(),
            workspace_id.to_string(),
            "--session".to_owned(),
            session_name,
        ]);
        let mut child = runtime
            .transport_mut(host_id)?
            .spawn_exec_with_stdin(&command)
            .map_err(|error| error.to_string())?;
        let stdin = child
            .take_stdin()
            .ok_or_else(|| "Session lease helper has no lifetime channel.".to_owned())?;
        let stdout = child
            .take_stdout()
            .ok_or_else(|| "Session lease helper has no ready channel.".to_owned())?;
        let ready = match read_ready_line(stdout) {
            Ok(ready) => ready,
            Err(error) => {
                drop(stdin);
                let _ = child.cancel();
                return Err(error);
            }
        };
        if ready.trim_end() != SESSION_LEASE_READY {
            drop(stdin);
            let output = child
                .wait_with_output()
                .map_err(|error| format!("Session lease helper failed: {error}"))?;
            let detail = String::from_utf8_lossy(&output.stderr);
            let detail = detail.trim();
            return Err(if detail.is_empty() {
                "Session lease helper exited before acquiring the lock.".to_owned()
            } else {
                format!("Session lease helper failed: {detail}")
            });
        }
        Ok(Self {
            stdin: Some(stdin),
            child: Some(child),
        })
    }

    pub(super) fn release(mut self) -> Result<(), String> {
        self.stdin.take();
        let child = self
            .child
            .take()
            .ok_or_else(|| "Session lease helper was already released.".to_owned())?;
        finish_release(child)
    }
}

fn read_ready_line(stdout: ChildStdout) -> Result<String, String> {
    if !crate::transport::CommandCancellation::scope_is_active() {
        let mut ready = String::new();
        BufReader::new(stdout)
            .take(128)
            .read_line(&mut ready)
            .map_err(|error| format!("Could not read the session lease handshake: {error}"))?;
        return Ok(ready);
    }

    read_ready_line_cancellable(stdout)
}

#[cfg(unix)]
fn read_ready_line_cancellable(mut stdout: ChildStdout) -> Result<String, String> {
    use std::os::fd::AsRawFd;

    const POLL_INTERVAL: Duration = Duration::from_millis(10);
    const MAX_BYTES: usize = 128;
    let descriptor = stdout.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err("Could not make the session lease handshake cancellable.".to_owned());
    }
    let mut bytes = Vec::new();
    loop {
        if crate::transport::CommandCancellation::scope_is_cancelled() {
            return Err(
                "Session restoration was cancelled while waiting for its lifecycle lease."
                    .to_owned(),
            );
        }
        let mut buffer = [0_u8; 128];
        let remaining = (MAX_BYTES - bytes.len()).min(buffer.len());
        match stdout.read(&mut buffer[..remaining]) {
            Ok(0) => break,
            Ok(count) => {
                bytes.extend_from_slice(&buffer[..count]);
                if bytes.contains(&b'\n') || bytes.len() == MAX_BYTES {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(error) => {
                return Err(format!(
                    "Could not read the session lease handshake: {error}"
                ));
            }
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| "Session lease helper returned a non-UTF-8 handshake.".to_owned())
}

#[cfg(not(unix))]
fn read_ready_line_cancellable(stdout: ChildStdout) -> Result<String, String> {
    let mut ready = String::new();
    BufReader::new(stdout)
        .take(128)
        .read_line(&mut ready)
        .map_err(|error| format!("Could not read the session lease handshake: {error}"))?;
    Ok(ready)
}

fn finish_release(mut child: RunningCommand) -> Result<(), String> {
    for _ in 0..RELEASE_POLL_ATTEMPTS {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|error| format!("Could not release the session lease: {error}"))?;
                return released_output(output);
            }
            Ok(None) => std::thread::sleep(RELEASE_POLL_INTERVAL),
            Err(error) => {
                let cancellation = child.cancel();
                return Err(match cancellation {
                    Ok(_) => format!("Could not poll the session lease helper: {error}"),
                    Err(cancel_error) => format!(
                        "Could not poll or cancel the session lease helper: {error}; {cancel_error}"
                    ),
                });
            }
        }
    }
    child.cancel().map(|_| ()).map_err(|error| {
        format!("Session lease release timed out and cancellation failed: {error}")
    })
}

fn released_output(output: crate::transport::CommandOutput) -> Result<(), String> {
    if output.success {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr);
        Err(if detail.trim().is_empty() {
            "Session lease helper failed while releasing the lock.".to_owned()
        } else {
            format!(
                "Session lease helper failed while releasing the lock: {}",
                detail.trim()
            )
        })
    }
}

impl Drop for SessionInitializationLease {
    fn drop(&mut self) {
        self.stdin.take();
        let Some(child) = self.child.take() else {
            return;
        };
        let _ = finish_release(child);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::transport::{CommandCancellation, ProcessSpec};
    use std::time::Instant;

    #[test]
    fn blocked_lease_handshake_is_cancellable() {
        let mut child =
            RunningCommand::spawn(&ProcessSpec::new("sh").args(["-c", "exec sleep 30"]), true)
                .unwrap();
        let stdout = child.take_stdout().unwrap();
        let cancellation = CommandCancellation::default();
        let worker_cancellation = cancellation.clone();
        let worker = std::thread::spawn(move || {
            let result = worker_cancellation.scoped(|| read_ready_line(stdout));
            let _ = child.cancel();
            result
        });
        std::thread::sleep(Duration::from_millis(30));
        let started = Instant::now();
        cancellation.cancel();
        let error = worker.join().unwrap().unwrap_err();

        assert!(error.contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
