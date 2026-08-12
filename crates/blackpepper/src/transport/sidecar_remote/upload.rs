//! Bounded file streaming shared by every managed remote binary install.

use super::SidecarInstallError;
use crate::transport::{CommandCancellation, CommandOutput, RunningCommand};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

const UPLOAD_BUFFER_BYTES: usize = 256 * 1024;
pub(super) const UPLOAD_STALL_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const UPLOAD_TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
const UPLOAD_WATCH_INTERVAL: Duration = Duration::from_millis(250);
const UPLOAD_WRITE_RETRY: Duration = Duration::from_millis(10);
const UPLOAD_EOF_TIMEOUT: Duration = Duration::from_secs(15);

/// Stream one local file into an already-spawned host command. The caller
/// remains responsible for remote checksum verification and atomic publish.
pub(crate) fn upload_file_to_child(
    mut child: RunningCommand,
    local_path: &Path,
) -> Result<CommandOutput, SidecarInstallError> {
    let local = File::open(local_path).map_err(|source| SidecarInstallError::Io {
        operation: format!("failed to open {}", local_path.display()),
        source,
    })?;
    let total = local
        .metadata()
        .map_err(|source| SidecarInstallError::Io {
            operation: format!("failed to inspect {}", local_path.display()),
            source,
        })?
        .len();
    let stdin = child
        .take_stdin()
        .ok_or(SidecarInstallError::MissingUploadStdin)?;
    let child = stream_upload(child, local, stdin, total)?;
    child
        .wait_with_output_timeout(UPLOAD_EOF_TIMEOUT)
        .map_err(Into::into)
}

fn stream_upload(
    child: RunningCommand,
    mut local: File,
    mut stdin: std::process::ChildStdin,
    total: u64,
) -> Result<RunningCommand, SidecarInstallError> {
    make_upload_pipe_nonblocking(&stdin).map_err(|source| SidecarInstallError::Io {
        operation: "failed to make the managed sidecar upload cancellable".to_owned(),
        source,
    })?;
    let progress = Arc::new(AtomicU64::new(0));
    let abort = Arc::new(AtomicBool::new(false));
    let worker_progress = Arc::clone(&progress);
    let worker_abort = Arc::clone(&abort);
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name("bp-sidecar-upload".to_string())
        .spawn(move || {
            let result =
                copy_with_progress(&mut local, &mut stdin, &worker_progress, &worker_abort)
                    .and_then(|_| stdin.flush());
            drop(stdin);
            let _ = result_tx.send(result);
        })
        .map_err(|source| SidecarInstallError::Io {
            operation: "failed to start the managed sidecar upload worker".to_string(),
            source,
        })?;

    watch_upload(child, worker, result_rx, progress, abort, total)
}

fn watch_upload(
    child: RunningCommand,
    worker: thread::JoinHandle<()>,
    result_rx: mpsc::Receiver<io::Result<()>>,
    progress: Arc<AtomicU64>,
    abort: Arc<AtomicBool>,
    total: u64,
) -> Result<RunningCommand, SidecarInstallError> {
    let started = Instant::now();
    let mut last_progress = started;
    let mut last_transferred = 0;
    loop {
        match result_rx.recv_timeout(UPLOAD_WATCH_INTERVAL) {
            Ok(Ok(())) => {
                let _ = worker.join();
                return Ok(child);
            }
            Ok(Err(source)) => {
                let _ = worker.join();
                return Err(SidecarInstallError::Io {
                    operation: "failed to stream sidecar to the workspace host".to_string(),
                    source,
                });
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = worker.join();
                return Err(SidecarInstallError::Io {
                    operation: "managed sidecar upload worker exited without a result".to_string(),
                    source: io::Error::other("upload worker channel closed"),
                });
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        let transferred = progress.load(Ordering::Relaxed);
        if transferred != last_transferred {
            last_transferred = transferred;
            last_progress = Instant::now();
        }
        let timed_out = started.elapsed() >= UPLOAD_TOTAL_TIMEOUT;
        let stalled = last_progress.elapsed() >= UPLOAD_STALL_TIMEOUT;
        let cancelled = CommandCancellation::scope_is_cancelled();
        if timed_out || stalled || cancelled {
            // Wake the nonblocking writer before cancelling the child. This
            // remains bounded even if a remote descendant inherited stdin.
            abort.store(true, Ordering::Release);
            let cancellation_error = child.cancel().err().map(|error| error.to_string());
            let _ = worker.join();
            return Err(upload_interrupted(
                cancelled,
                timed_out,
                transferred,
                total,
                cancellation_error,
            ));
        }
    }
}

fn upload_interrupted(
    cancelled: bool,
    timed_out: bool,
    transferred: u64,
    total: u64,
    cancellation_error: Option<String>,
) -> SidecarInstallError {
    if cancelled {
        SidecarInstallError::UploadCancelled {
            transferred,
            total,
            cancellation_error,
        }
    } else if timed_out {
        SidecarInstallError::UploadTimedOut {
            transferred,
            total,
            cancellation_error,
        }
    } else {
        SidecarInstallError::UploadStalled {
            transferred,
            total,
            cancellation_error,
        }
    }
}

fn copy_with_progress(
    reader: &mut impl Read,
    writer: &mut impl Write,
    progress: &AtomicU64,
    abort: &AtomicBool,
) -> io::Result<u64> {
    let mut buffer = vec![0_u8; UPLOAD_BUFFER_BYTES];
    let mut transferred = 0_u64;
    loop {
        if abort.load(Ordering::Acquire) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "upload aborted"));
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(transferred);
        }
        let mut offset = 0;
        while offset < read {
            if abort.load(Ordering::Acquire) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "upload aborted"));
            }
            match writer.write(&buffer[offset..read]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(written) => {
                    offset += written;
                    transferred = transferred.saturating_add(written as u64);
                    progress.store(transferred, Ordering::Relaxed);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(UPLOAD_WRITE_RETRY);
                }
                Err(error) => return Err(error),
            }
        }
    }
}

#[cfg(unix)]
fn make_upload_pipe_nonblocking(pipe: &std::process::ChildStdin) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    let descriptor = pipe.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
fn make_upload_pipe_nonblocking(_pipe: &std::process::ChildStdin) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
#[path = "upload_tests.rs"]
mod tests;
