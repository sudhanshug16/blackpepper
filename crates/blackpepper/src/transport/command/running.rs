use super::{CommandOutput, ProcessSpec};
use crate::transport::TransportError;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Stdio};

/// An owned child that is terminated if its handle is abandoned.
#[derive(Debug)]
pub struct RunningCommand {
    pub(super) child: Option<Child>,
    cancel_spec: Option<ProcessSpec>,
    kill_process_group: bool,
}

impl RunningCommand {
    pub(crate) fn spawn(spec: &ProcessSpec, pipe_stdin: bool) -> Result<Self, TransportError> {
        Self::spawn_with_group_policy(spec, pipe_stdin, false)
    }

    /// Spawn one local supervisor as a private process group so cancelling the
    /// supervisor also reaps bounded observation children it has started.
    #[cfg(unix)]
    pub(crate) fn spawn_in_process_group(
        spec: &ProcessSpec,
        pipe_stdin: bool,
    ) -> Result<Self, TransportError> {
        Self::spawn_with_group_policy(spec, pipe_stdin, true)
    }

    fn spawn_with_group_policy(
        spec: &ProcessSpec,
        pipe_stdin: bool,
        kill_process_group: bool,
    ) -> Result<Self, TransportError> {
        let mut command = spec.to_command();
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        if pipe_stdin {
            command.stdin(Stdio::piped());
        } else {
            command.stdin(Stdio::null());
        }
        #[cfg(unix)]
        if kill_process_group {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = command
            .spawn()
            .map_err(|source| TransportError::io("failed to spawn command", source))?;
        Ok(Self {
            child: Some(child),
            cancel_spec: None,
            kill_process_group,
        })
    }

    pub(crate) fn spawn_with_cancel(
        spec: &ProcessSpec,
        pipe_stdin: bool,
        cancel_spec: ProcessSpec,
    ) -> Result<Self, TransportError> {
        let mut command = Self::spawn(spec, pipe_stdin)?;
        command.cancel_spec = Some(cancel_spec);
        Ok(command)
    }

    pub fn id(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.as_mut()?.stdin.take()
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.as_mut()?.stdout.take()
    }

    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.as_mut()?.stderr.take()
    }

    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, TransportError> {
        self.child
            .as_mut()
            .ok_or_else(|| TransportError::InvalidCommand("command already reaped".to_string()))?
            .try_wait()
            .map_err(|source| TransportError::io("failed to poll command", source))
    }

    pub fn wait_with_output(self) -> Result<CommandOutput, TransportError> {
        if crate::transport::cancellation::requested() {
            return self.cancelled_error();
        }
        if crate::transport::cancellation::active() {
            return self.wait_with_output_scoped();
        }
        self.wait_with_output_unchecked()
    }

    pub(super) fn wait_with_output_unchecked(mut self) -> Result<CommandOutput, TransportError> {
        let child = self
            .child
            .take()
            .ok_or_else(|| TransportError::InvalidCommand("command already reaped".to_string()))?;
        child
            .wait_with_output()
            .map(CommandOutput::from)
            .map_err(|source| TransportError::io("failed to wait for command", source))
    }

    pub(super) fn cancelled_error(self) -> Result<CommandOutput, TransportError> {
        let process_id = self.id().unwrap_or_default();
        let cancellation_error = self.cancel().err().map(|error| error.to_string());
        Err(TransportError::CommandCancelled {
            process_id,
            cancellation_error,
        })
    }

    pub fn cancel(mut self) -> Result<CommandOutput, TransportError> {
        let child = self
            .child
            .take()
            .ok_or_else(|| TransportError::InvalidCommand("command already reaped".to_string()))?;
        crate::transport::process_cancel::cancel_child(
            child,
            self.cancel_spec.take(),
            self.kill_process_group,
        )
    }
}

impl Drop for RunningCommand {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            let _ = crate::transport::process_cancel::cancel_child(
                child,
                self.cancel_spec.take(),
                self.kill_process_group,
            );
        }
    }
}
