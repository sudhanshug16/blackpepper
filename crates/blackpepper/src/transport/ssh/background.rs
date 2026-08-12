use super::SshTransport;
use crate::transport::{HostCommand, LocalForward, ProcessSpec, RunningCommand, TransportError};

impl SshTransport {
    /// Spawn a read-only background observation after checking only the owned
    /// master process and socket handle. The child remains fail-closed through
    /// its `ControlMaster=no`, `ProxyJump=none`, and `ProxyCommand=false` argv;
    /// a stale socket therefore fails inside the worker instead of making the
    /// render thread wait for `ssh -O check`.
    pub(crate) fn spawn_background_exec_with_stdin(
        &mut self,
        command: &HostCommand,
    ) -> Result<RunningCommand, TransportError> {
        self.preflight_master_handle()?;
        let (command_spec, cancel_spec) = crate::transport::ssh_cancel::cancellable_session_specs(
            &self.config,
            self.socket()?,
            command,
        )?;
        RunningCommand::spawn_with_cancel(&command_spec, true, cancel_spec)
    }

    /// Prepare an owned `ssh -O cancel` command without waiting for it. The
    /// caller runs this exact spec on a background worker and confirms success
    /// before removing transport ownership.
    pub(crate) fn background_cancel_spec(
        &self,
        forward: &LocalForward,
    ) -> Result<ProcessSpec, TransportError> {
        self.ensure_ready()?;
        if !self.forwards.contains(forward) {
            return Err(TransportError::ForwardNotOwned(forward.clone()));
        }
        crate::transport::ssh_command::control_spec(
            &self.config,
            self.socket()?,
            crate::transport::ssh_command::ControlAction::Cancel(forward),
        )
    }

    pub(crate) fn confirm_background_cancel(&mut self, forward: &LocalForward) {
        self.forwards.remove(forward);
    }
}
