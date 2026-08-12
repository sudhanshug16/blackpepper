use super::*;
use crate::transport::{
    CommandOutput, HostKind, LocalForward, PtyProcess, RunningCommand, TransportError,
};
use portable_pty::PtySize;
use std::sync::{mpsc, Arc, Barrier};
use std::time::Duration;

struct StalledLookup {
    entered: Arc<Barrier>,
    release: Arc<Barrier>,
}

impl HostTransport for StalledLookup {
    fn kind(&self) -> HostKind {
        HostKind::Local
    }

    fn spawn_exec(&mut self, _command: &HostCommand) -> Result<RunningCommand, TransportError> {
        Err(TransportError::Unsupported("not used"))
    }

    fn spawn_exec_with_stdin(
        &mut self,
        _command: &HostCommand,
    ) -> Result<RunningCommand, TransportError> {
        Err(TransportError::Unsupported("not used"))
    }

    fn exec(&mut self, _command: &HostCommand) -> Result<CommandOutput, TransportError> {
        self.entered.wait();
        self.release.wait();
        Err(TransportError::Unsupported("fixture stopped after lookup"))
    }

    fn exec_timeout(
        &mut self,
        command: &HostCommand,
        _timeout: Duration,
    ) -> Result<CommandOutput, TransportError> {
        self.exec(command)
    }

    fn attach_pty(
        &mut self,
        _command: &HostCommand,
        _size: PtySize,
    ) -> Result<PtyProcess, TransportError> {
        Err(TransportError::Unsupported("not used"))
    }

    fn forward_local_port(
        &mut self,
        _forward: LocalForward,
    ) -> Result<LocalForward, TransportError> {
        Err(TransportError::Unsupported("not used"))
    }

    fn cancel_local_forward(&mut self, _forward: &LocalForward) -> Result<(), TransportError> {
        Err(TransportError::Unsupported("not used"))
    }
}

#[test]
fn stalled_first_use_helper_lookup_does_not_block_raw_input_channel() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let worker_entered = Arc::clone(&entered);
    let worker_release = Arc::clone(&release);
    let worker = std::thread::spawn(move || {
        let mut transport = StalledLookup {
            entered: worker_entered,
            release: worker_release,
        };
        find_helper_with(&mut transport, |_| {
            Err("fixture should not package".to_owned())
        })
    });
    entered.wait();
    let (events, receiver) = mpsc::channel();

    events
        .send(crate::client::ClientEvent::RawInput(vec![b'x']))
        .unwrap();
    assert!(matches!(
        receiver.recv().unwrap(),
        crate::client::ClientEvent::RawInput(bytes) if bytes == b"x"
    ));

    release.wait();
    assert!(worker.join().unwrap().is_err());
}
