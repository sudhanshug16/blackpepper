use std::collections::VecDeque;
use std::time::Duration;

use portable_pty::PtySize;

use crate::transport::{
    CommandOutput, HostCommand, HostKind, HostTransport, LocalForward, PtyProcess, RunningCommand,
    TransportError,
};

pub(super) fn metadata(
    uid: &str,
    temporary: &str,
    xdg: &str,
    internal: &str,
    native: &str,
) -> CommandOutput {
    success(&format!(
        "{uid}\0{temporary}\0{xdg}\0{internal}\0{native}\0"
    ))
}

pub(super) fn active_session() -> CommandOutput {
    success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n")
}

pub(super) fn missing_no_sessions() -> CommandOutput {
    CommandOutput {
        success: false,
        status: Some(1),
        stdout: Vec::new(),
        stderr: b"There is no active session!\n".to_vec(),
    }
}

pub(super) fn missing_socket() -> CommandOutput {
    CommandOutput {
        success: false,
        status: Some(3),
        stdout: Vec::new(),
        stderr: Vec::new(),
    }
}

pub(super) fn physical_directory(path: &str) -> CommandOutput {
    success(&format!("{path}\n"))
}

pub(super) fn missing_named(session: &str) -> CommandOutput {
    CommandOutput {
        success: true,
        status: Some(0),
        stdout: b"some-other-session\n".to_vec(),
        stderr: format!("Session '{session}' not found. The following sessions are active:\n")
            .into_bytes(),
    }
}

pub(super) fn success(stdout: &str) -> CommandOutput {
    CommandOutput {
        success: true,
        status: Some(0),
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
    }
}

pub(super) fn socket_directory(command: &HostCommand) -> &str {
    command
        .env
        .get("ZELLIJ_SOCKET_DIR")
        .expect("resolved commands pin the socket directory")
}

pub(super) fn zellij_arguments(command: &HostCommand) -> &[String] {
    &command.args
}

pub(super) struct ScriptedTransport {
    pub(super) outputs: VecDeque<CommandOutput>,
    pub(super) commands: Vec<HostCommand>,
}

impl ScriptedTransport {
    pub(super) fn new(outputs: impl IntoIterator<Item = CommandOutput>) -> Self {
        Self {
            outputs: outputs.into_iter().collect(),
            commands: Vec::new(),
        }
    }
}

impl HostTransport for ScriptedTransport {
    fn kind(&self) -> HostKind {
        HostKind::Local
    }

    fn spawn_exec(&mut self, _command: &HostCommand) -> Result<RunningCommand, TransportError> {
        Err(TransportError::Unsupported("not used by this test"))
    }

    fn spawn_exec_with_stdin(
        &mut self,
        _command: &HostCommand,
    ) -> Result<RunningCommand, TransportError> {
        Err(TransportError::Unsupported("not used by this test"))
    }

    fn attach_pty(
        &mut self,
        _command: &HostCommand,
        _size: PtySize,
    ) -> Result<PtyProcess, TransportError> {
        Err(TransportError::Unsupported("not used by this test"))
    }

    fn forward_local_port(
        &mut self,
        _forward: LocalForward,
    ) -> Result<LocalForward, TransportError> {
        Err(TransportError::Unsupported("not used by this test"))
    }

    fn cancel_local_forward(&mut self, _forward: &LocalForward) -> Result<(), TransportError> {
        Err(TransportError::Unsupported("not used by this test"))
    }

    fn exec(&mut self, command: &HostCommand) -> Result<CommandOutput, TransportError> {
        self.commands.push(command.clone());
        self.outputs
            .pop_front()
            .ok_or(TransportError::Unsupported("unexpected command"))
    }

    fn exec_timeout(
        &mut self,
        command: &HostCommand,
        _timeout: Duration,
    ) -> Result<CommandOutput, TransportError> {
        self.exec(command)
    }
}
