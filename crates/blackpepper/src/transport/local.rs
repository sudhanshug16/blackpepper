use portable_pty::PtySize;

use super::{
    HostCommand, HostKind, HostTransport, LocalForward, ProcessSpec, PtyProcess, RunningCommand,
    TransportError,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalTransport;

impl LocalTransport {
    pub fn process_spec(command: &HostCommand) -> Result<ProcessSpec, TransportError> {
        command.validate()?;
        let mut spec = ProcessSpec::new(&command.program).args(command.args.iter().cloned());
        if let Some(cwd) = &command.cwd {
            spec = spec.cwd(cwd);
        }
        for (key, value) in &command.env {
            spec = spec.env(key, value);
        }
        Ok(spec)
    }
}

impl HostTransport for LocalTransport {
    fn kind(&self) -> HostKind {
        HostKind::Local
    }

    fn spawn_exec(&mut self, command: &HostCommand) -> Result<RunningCommand, TransportError> {
        RunningCommand::spawn(&Self::process_spec(command)?, false)
    }

    fn spawn_exec_with_stdin(
        &mut self,
        command: &HostCommand,
    ) -> Result<RunningCommand, TransportError> {
        RunningCommand::spawn(&Self::process_spec(command)?, true)
    }

    fn attach_pty(
        &mut self,
        command: &HostCommand,
        size: PtySize,
    ) -> Result<PtyProcess, TransportError> {
        PtyProcess::spawn(&Self::process_spec(command)?, size)
    }

    fn forward_local_port(
        &mut self,
        forward: LocalForward,
    ) -> Result<LocalForward, TransportError> {
        forward.validate()?;
        if forward.local_port != forward.remote_port {
            return Err(TransportError::InvalidForward(
                "local workspaces expose the service directly; local and service ports must match"
                    .to_string(),
            ));
        }
        Ok(forward)
    }

    fn cancel_local_forward(&mut self, forward: &LocalForward) -> Result<(), TransportError> {
        forward.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn process_spec_keeps_argv_separate() {
        let command = HostCommand::new("printf")
            .args(["%s", "hello world"])
            .cwd("/tmp")
            .env("COLOR", "always");
        let spec = LocalTransport::process_spec(&command).unwrap();

        assert_eq!(spec.program.to_string_lossy(), "printf");
        assert_eq!(spec.args, ["%s", "hello world"]);
        assert_eq!(spec.cwd.as_deref(), Some(std::path::Path::new("/tmp")));
        assert_eq!(
            spec.env.get(std::ffi::OsStr::new("COLOR")),
            Some(&"always".into())
        );
    }

    #[cfg(unix)]
    #[test]
    fn stdin_exec_channel_can_stream_helper_bytes() {
        let mut transport = LocalTransport;
        let mut child = transport
            .spawn_exec_with_stdin(&HostCommand::new("sh").args(["-c", "cat"]))
            .unwrap();
        let mut stdin = child.take_stdin().expect("piped stdin");
        stdin.write_all(b"sidecar bytes").unwrap();
        drop(stdin);

        let output = child.wait_with_output().unwrap();
        assert!(output.success);
        assert_eq!(output.stdout, b"sidecar bytes");
    }
}
