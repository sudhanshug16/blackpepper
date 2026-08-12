use std::process::{Child, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::{CommandOutput, TransportError};

const GRACEFUL_CANCEL_TIMEOUT: Duration = Duration::from_millis(500);
const FORCED_CANCEL_TIMEOUT: Duration = Duration::from_millis(500);
const REMOTE_CANCEL_TIMEOUT: Duration = Duration::from_millis(1_750);
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Cancel a local command without waiting indefinitely on mux-owned pipes.
///
/// OpenSSH multiplex clients pass their output descriptors to the foreground
/// master. Killing only the small mux child and then waiting for all output can
/// therefore block forever while a remote command keeps the master's copies
/// open. SSH commands may provide a second fail-closed mux command that stops
/// their validated remote PID first. The local child is then terminated after
/// bounded grace periods, with an asynchronous reaper as the final fallback.
pub(super) fn cancel_child(
    mut child: Child,
    mut cancel_spec: Option<super::ProcessSpec>,
    kill_process_group: bool,
) -> Result<CommandOutput, TransportError> {
    let process_id = child.id();

    // Closing stdin asks cooperative helpers to finish. Dropping the read ends
    // is essential: the foreground mux master may still own duplicate writers,
    // so waiting for their EOF would make cancellation unbounded.
    child.stdin.take();
    child.stdout.take();
    child.stderr.take();

    let initial_status = match child.try_wait() {
        Ok(status) => status,
        Err(source) => {
            if let Some(spec) = cancel_spec.take() {
                let _ = run_remote_cancel(&spec);
            }
            force_and_reap(child, kill_process_group);
            return Err(TransportError::io(
                "failed to poll command before cancellation",
                source,
            ));
        }
    };
    if let Some(status) = initial_status {
        force_group_gone(process_id, kill_process_group);
        return Ok(cancelled_output(status));
    }

    let remote_result = match cancel_spec {
        Some(spec) => run_remote_cancel(&spec),
        None => Ok(()),
    };

    request_graceful_termination(&mut child, kill_process_group);
    let graceful = match wait_bounded(&mut child, GRACEFUL_CANCEL_TIMEOUT) {
        Ok(status) => status,
        Err(error) => {
            force_and_reap(child, kill_process_group);
            return Err(error);
        }
    };
    if let Some(status) = graceful {
        force_group_gone(process_id, kill_process_group);
        return finish_cancel(status, remote_result);
    }

    // A local SIGKILL cannot be forwarded through SSH, but it guarantees that
    // this client-side mux process stops after the explicit remote cancellation
    // attempt. Descendants that deliberately escape the recorded main process
    // remain outside this transport's safe cancellation boundary.
    let kill_result = force_termination(&mut child, kill_process_group);
    let forced = match wait_bounded(&mut child, FORCED_CANCEL_TIMEOUT) {
        Ok(status) => status,
        Err(error) => {
            force_and_reap(child, kill_process_group);
            return Err(error);
        }
    };
    if let Some(status) = forced {
        force_group_gone(process_id, kill_process_group);
        return finish_cancel(status, remote_result);
    }
    if let Err(source) = kill_result {
        spawn_reaper(child);
        return Err(TransportError::io("failed to force-cancel command", source));
    }

    spawn_reaper(child);
    Err(TransportError::CancellationTimedOut { process_id })
}

fn wait_bounded(
    child: &mut Child,
    timeout: Duration,
) -> Result<Option<ExitStatus>, TransportError> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|source| TransportError::io("failed to poll cancelled command", source))?
        {
            return Ok(Some(status));
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        thread::sleep(CANCEL_POLL_INTERVAL.min(deadline.saturating_duration_since(now)));
    }
}

#[cfg(unix)]
fn request_graceful_termination(child: &mut Child, kill_process_group: bool) {
    // TERM gives ordinary local commands a chance to clean up. SSH remote
    // cancellation is handled separately because mux clients do not reliably
    // forward a locally delivered process signal to their remote session.
    unsafe {
        libc::kill(
            if kill_process_group {
                -(child.id() as libc::pid_t)
            } else {
                child.id() as libc::pid_t
            },
            libc::SIGTERM,
        );
    }
}

#[cfg(not(unix))]
fn request_graceful_termination(child: &mut Child, _kill_process_group: bool) {
    let _ = child.kill();
}

#[cfg(unix)]
fn force_termination(child: &mut Child, kill_process_group: bool) -> std::io::Result<()> {
    if !kill_process_group {
        return child.kill();
    }
    let result = unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL) };
    if result == 0 {
        Ok(())
    } else {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }
}

#[cfg(not(unix))]
fn force_termination(child: &mut Child, _kill_process_group: bool) -> std::io::Result<()> {
    child.kill()
}

#[cfg(unix)]
fn force_group_gone(process_id: u32, kill_process_group: bool) {
    if kill_process_group {
        unsafe {
            libc::kill(-(process_id as libc::pid_t), libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn force_group_gone(_process_id: u32, _kill_process_group: bool) {}

fn spawn_reaper(mut child: Child) {
    let _ = thread::Builder::new()
        .name("bp-process-reaper".to_string())
        .spawn(move || {
            let _ = child.kill();
            let _ = child.wait();
        });
}

fn force_and_reap(mut child: Child, kill_process_group: bool) {
    request_graceful_termination(&mut child, kill_process_group);
    let _ = force_termination(&mut child, kill_process_group);
    spawn_reaper(child);
}

fn run_remote_cancel(spec: &super::ProcessSpec) -> Result<(), TransportError> {
    let mut command = spec.to_command();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|source| TransportError::io("failed to start remote cancellation", source))?;
    let process_id = child.id();
    let remote_wait = match wait_bounded(&mut child, REMOTE_CANCEL_TIMEOUT) {
        Ok(status) => status,
        Err(error) => {
            force_and_reap(child, false);
            return Err(error);
        }
    };
    match remote_wait {
        Some(status) if status.success() => Ok(()),
        Some(status) => Err(TransportError::CommandFailed {
            operation: "remote command cancellation".to_string(),
            status: status.code(),
            stderr: String::new(),
        }),
        None => {
            let _ = child.kill();
            match wait_bounded(&mut child, FORCED_CANCEL_TIMEOUT) {
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => spawn_reaper(child),
            }
            Err(TransportError::CancellationTimedOut { process_id })
        }
    }
}

fn finish_cancel(
    status: ExitStatus,
    remote_result: Result<(), TransportError>,
) -> Result<CommandOutput, TransportError> {
    remote_result?;
    Ok(cancelled_output(status))
}

fn cancelled_output(status: ExitStatus) -> CommandOutput {
    CommandOutput {
        success: status.success(),
        status: status.code(),
        stdout: Vec::new(),
        stderr: Vec::new(),
    }
}
