use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use portable_pty::PtySize;

use super::ssh_command::{self, ControlAction};
use super::{
    CommandOutput, ControlSocket, HostCommand, HostKind, HostTransport, LocalForward, ProcessSpec,
    PtyProcess, RunningCommand, SshConfig, TransportError,
};

mod background;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Ready,
    Failed { status: Option<u32> },
}

pub struct SshTransport {
    config: SshConfig,
    socket: Option<ControlSocket>,
    master: Option<PtyProcess>,
    state: ConnectionState,
    forwards: BTreeSet<LocalForward>,
}

impl fmt::Debug for SshTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SshTransport")
            .field("destination", &self.config.destination)
            .field("socket", &self.socket)
            .field("state", &self.state)
            .field("forwards", &self.forwards)
            .finish_non_exhaustive()
    }
}

impl SshTransport {
    pub fn new(config: SshConfig) -> Result<Self, TransportError> {
        config.validate()?;
        Ok(Self {
            config,
            socket: None,
            master: None,
            state: ConnectionState::Disconnected,
            forwards: BTreeSet::new(),
        })
    }

    pub fn config(&self) -> &SshConfig {
        &self.config
    }

    pub fn state(&self) -> &ConnectionState {
        &self.state
    }

    pub fn control_socket_path(&self) -> Option<&Path> {
        self.socket.as_ref().map(ControlSocket::path)
    }

    pub fn master_pty_mut(&mut self) -> Option<&mut PtyProcess> {
        self.master.as_mut()
    }

    pub fn start_master(&mut self, size: PtySize) -> Result<(), TransportError> {
        if matches!(
            self.state,
            ConnectionState::Connecting | ConnectionState::Ready
        ) {
            return Err(TransportError::AlreadyConnected);
        }
        self.force_stop();
        let socket = ControlSocket::allocate(self.config.control_root.as_deref())?;
        let spec = ssh_command::master_spec(&self.config, &socket)?;
        let master = PtyProcess::spawn(&spec, size)?;
        self.socket = Some(socket);
        self.master = Some(master);
        self.state = ConnectionState::Connecting;
        Ok(())
    }

    /// Poll both the foreground master process and its control socket.
    pub fn poll_connection(&mut self) -> Result<ConnectionState, TransportError> {
        let was_ready = self.state == ConnectionState::Ready;
        let Some(master) = self.master.as_mut() else {
            self.state = ConnectionState::Disconnected;
            return Ok(self.state.clone());
        };
        if let Some(exit) = master.try_wait()? {
            self.state = ConnectionState::Failed {
                status: Some(exit.code),
            };
            self.forwards.clear();
            return Ok(self.state.clone());
        }

        // OpenSSH only creates the control socket after authentication. Avoid
        // spawning repeated `ssh -O check` children while a prompt is open.
        if !self.socket()?.path().exists() {
            self.state = if was_ready {
                self.forwards.clear();
                ConnectionState::Failed { status: None }
            } else {
                ConnectionState::Connecting
            };
            return Ok(self.state.clone());
        }

        // The foreground ControlMaster child and the socket it created are the
        // non-blocking readiness signal. Do not run `ssh -O check` here: this
        // method is called by the render thread, and even the one-time check
        // can stall on a broken SSH binary or filesystem. The first helper
        // channel performs the normal fail-closed mux preflight inside the
        // generation-owned restore worker before any remote state is trusted.
        self.state = ConnectionState::Ready;
        Ok(self.state.clone())
    }

    fn build_exec_specs(
        &mut self,
        command: &HostCommand,
    ) -> Result<(ProcessSpec, ProcessSpec), TransportError> {
        self.preflight_master()?;
        super::ssh_cancel::cancellable_session_specs(&self.config, self.socket()?, command)
    }

    fn build_pty_spec(&mut self, command: &HostCommand) -> Result<ProcessSpec, TransportError> {
        self.preflight_master()?;
        ssh_command::session_spec(&self.config, self.socket()?, command, true)
    }

    pub fn disconnect(&mut self) -> Result<(), TransportError> {
        // The foreground master is Blackpepper-owned. Tear it down first so
        // a wedged `ssh -O exit` can never hold the TUI/render thread. The
        // socket directory is removed with the owned control handle.
        self.force_stop();
        Ok(())
    }

    fn run_control(&self, action: ControlAction<'_>) -> Result<CommandOutput, TransportError> {
        let spec = ssh_command::control_spec(&self.config, self.socket()?, action)?;
        RunningCommand::spawn(&spec, false)?.wait_with_output()
    }

    fn socket(&self) -> Result<&ControlSocket, TransportError> {
        self.socket.as_ref().ok_or(TransportError::NotConnected)
    }

    fn ensure_ready(&self) -> Result<(), TransportError> {
        match self.state {
            ConnectionState::Ready => Ok(()),
            ConnectionState::Failed { status } => Err(TransportError::MasterExited(status)),
            _ => Err(TransportError::NotConnected),
        }
    }

    /// Check the owned mux immediately before starting an ordinary channel.
    ///
    /// The session argv also poisons direct-connection fallback, which closes
    /// the remaining check-to-spawn race if the master exits between these two
    /// operations.
    fn preflight_master(&mut self) -> Result<(), TransportError> {
        self.preflight_master_handle()?;
        let output = self.run_control(ControlAction::Check)?;
        if output.success {
            return Ok(());
        }
        self.state = ConnectionState::Failed { status: None };
        self.forwards.clear();
        Err(command_failure("SSH control-master preflight", output))
    }

    fn preflight_master_handle(&mut self) -> Result<(), TransportError> {
        self.ensure_ready()?;
        if let Some(exit) = self
            .master
            .as_mut()
            .ok_or(TransportError::NotConnected)?
            .try_wait()?
        {
            self.state = ConnectionState::Failed {
                status: Some(exit.code),
            };
            self.forwards.clear();
            return Err(TransportError::MasterExited(Some(exit.code)));
        }
        if !self.socket()?.path().exists() {
            self.state = ConnectionState::Failed { status: None };
            self.forwards.clear();
            return Err(TransportError::MasterExited(None));
        }
        Ok(())
    }

    fn force_stop(&mut self) {
        if let Some(master) = self.master.as_mut() {
            let running = !matches!(master.try_wait(), Ok(Some(_)));
            if running {
                let _ = master.kill();
                let _ = master.wait();
            }
        }
        self.master.take();
        self.socket.take();
        self.forwards.clear();
        self.state = ConnectionState::Disconnected;
    }
}

impl HostTransport for SshTransport {
    fn kind(&self) -> HostKind {
        HostKind::Ssh
    }

    fn spawn_exec(&mut self, command: &HostCommand) -> Result<RunningCommand, TransportError> {
        let (command_spec, cancel_spec) = self.build_exec_specs(command)?;
        RunningCommand::spawn_with_cancel(&command_spec, false, cancel_spec)
    }

    fn spawn_exec_with_stdin(
        &mut self,
        command: &HostCommand,
    ) -> Result<RunningCommand, TransportError> {
        let (command_spec, cancel_spec) = self.build_exec_specs(command)?;
        RunningCommand::spawn_with_cancel(&command_spec, true, cancel_spec)
    }

    fn attach_pty(
        &mut self,
        command: &HostCommand,
        size: PtySize,
    ) -> Result<PtyProcess, TransportError> {
        PtyProcess::spawn(&self.build_pty_spec(command)?, size)
    }

    fn forward_local_port(
        &mut self,
        forward: LocalForward,
    ) -> Result<LocalForward, TransportError> {
        self.ensure_ready()?;
        forward.validate()?;
        if self.forwards.contains(&forward) {
            return Ok(forward);
        }
        let output = self.run_control(ControlAction::Forward(&forward))?;
        if !output.success {
            return Err(command_failure("SSH port forwarding", output));
        }
        self.forwards.insert(forward.clone());
        Ok(forward)
    }

    fn cancel_local_forward(&mut self, forward: &LocalForward) -> Result<(), TransportError> {
        self.ensure_ready()?;
        if !self.forwards.contains(forward) {
            return Err(TransportError::ForwardNotOwned(forward.clone()));
        }
        let output = self.run_control(ControlAction::Cancel(forward))?;
        if !output.success {
            return Err(command_failure("SSH port-forward cancellation", output));
        }
        self.forwards.remove(forward);
        Ok(())
    }
}

impl Drop for SshTransport {
    fn drop(&mut self) {
        self.force_stop();
    }
}

fn command_failure(operation: &str, output: CommandOutput) -> TransportError {
    TransportError::CommandFailed {
        operation: operation.to_string(),
        status: output.status,
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}
