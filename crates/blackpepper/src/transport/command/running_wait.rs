use super::running::RunningCommand;
use super::CommandOutput;
use crate::transport::TransportError;
use std::thread;
use std::time::{Duration, Instant};

impl RunningCommand {
    #[cfg(unix)]
    pub(super) fn wait_with_output_scoped(mut self) -> Result<CommandOutput, TransportError> {
        use std::os::fd::AsRawFd;

        const POLL_INTERVAL: Duration = Duration::from_millis(10);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| TransportError::InvalidCommand("command already reaped".to_string()))?;
        if let Some(pipe) = child.stdout.as_ref() {
            set_nonblocking(pipe.as_raw_fd())?;
        }
        if let Some(pipe) = child.stderr.as_ref() {
            set_nonblocking(pipe.as_raw_fd())?;
        }
        loop {
            let child = self.child.as_mut().expect("scoped command disappeared");
            drain_available(child.stdout.as_mut(), &mut stdout)?;
            drain_available(child.stderr.as_mut(), &mut stderr)?;
            if let Some(status) = child
                .try_wait()
                .map_err(|source| TransportError::io("failed to poll command", source))?
            {
                // Drop pipe readers after draining bytes already available;
                // unrelated descendants may have inherited their write ends.
                drain_available(child.stdout.as_mut(), &mut stdout)?;
                drain_available(child.stderr.as_mut(), &mut stderr)?;
                self.child.take();
                return Ok(CommandOutput {
                    success: status.success(),
                    status: status.code(),
                    stdout,
                    stderr,
                });
            }
            if crate::transport::cancellation::requested() {
                return self.cancelled_error();
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    #[cfg(not(unix))]
    pub(super) fn wait_with_output_scoped(self) -> Result<CommandOutput, TransportError> {
        self.wait_with_output_unchecked()
    }

    /// Wait for bounded-output metadata with a deadline and scoped shutdown.
    pub fn wait_with_output_timeout(
        mut self,
        timeout: Duration,
    ) -> Result<CommandOutput, TransportError> {
        const POLL_INTERVAL: Duration = Duration::from_millis(10);

        let process_id = self
            .id()
            .ok_or_else(|| TransportError::InvalidCommand("command already reaped".to_string()))?;
        let started = Instant::now();
        loop {
            if self.try_wait()?.is_some() {
                // `Child::wait_with_output` can still wait for EOF from an
                // unrelated descendant that inherited the pipe. On Unix,
                // use the bounded nonblocking drain even though the command
                // itself has already exited.
                #[cfg(unix)]
                return self.wait_with_output_scoped();
                #[cfg(not(unix))]
                return self.wait_with_output();
            }
            // Cancellation wins over a simultaneous deadline so connection
            // shutdown is reported as cancellation rather than a false stall.
            if crate::transport::cancellation::requested() {
                return self.cancelled_error();
            }
            if started.elapsed() >= timeout {
                let cancellation_error = self.cancel().err().map(|error| error.to_string());
                return Err(TransportError::CommandTimedOut {
                    process_id,
                    timeout_ms: timeout.as_millis().try_into().unwrap_or(u64::MAX),
                    cancellation_error,
                });
            }
            thread::sleep(POLL_INTERVAL.min(timeout.saturating_sub(started.elapsed())));
        }
    }
}

#[cfg(unix)]
fn set_nonblocking(descriptor: std::os::fd::RawFd) -> Result<(), TransportError> {
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(TransportError::io(
            "failed to make command output cancellable",
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn drain_available(
    pipe: Option<&mut impl std::io::Read>,
    output: &mut Vec<u8>,
) -> Result<(), TransportError> {
    const MAX_READS_PER_PASS: usize = 8;
    let Some(pipe) = pipe else {
        return Ok(());
    };
    let mut buffer = [0_u8; 8192];
    for _ in 0..MAX_READS_PER_PASS {
        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => output.extend_from_slice(&buffer[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(TransportError::io(
                    "failed to read cancellable command output",
                    error,
                ));
            }
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
#[path = "running_wait_tests.rs"]
mod tests;
