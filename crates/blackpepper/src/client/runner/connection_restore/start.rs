use super::{Coordinator, RestoreWorker};
use crate::client::runtime::{ClientRuntime, ConnectionRestoreRuntime};
use crate::client::{ClientEvent, ClientState, HostConnection};
use crate::core::HostId;
use crate::transport::CommandCancellation;
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

impl Coordinator {
    pub(in crate::client::runner) fn start(
        &mut self,
        state: &mut ClientState,
        runtime: &mut ClientRuntime,
        host_id: HostId,
        events: &Sender<ClientEvent>,
    ) {
        self.start_with_spawner(state, runtime, host_id, events, |worker| {
            std::thread::Builder::new()
                .name(format!("bp-restore-{host_id}"))
                .spawn(worker)
        });
    }

    pub(super) fn start_with_spawner(
        &mut self,
        state: &mut ClientState,
        runtime: &mut ClientRuntime,
        host_id: HostId,
        events: &Sender<ClientEvent>,
        spawn: impl FnOnce(RestoreWorker) -> std::io::Result<JoinHandle<()>>,
    ) {
        self.invalidate(host_id);
        let (generation, token) = self.gate.begin(host_id);
        let cancellation = CommandCancellation::default();
        let worker_cancellation = cancellation.clone();
        let forwards = state
            .forwards
            .iter()
            .filter(|forward| forward.host_id == host_id)
            .cloned()
            .collect();
        let attached = state
            .terminals
            .keys()
            .copied()
            .filter(|id| state.host_for_workspace(*id) == Some(host_id))
            .collect();
        let (outcome_tx, outcome_rx) = mpsc::sync_channel(1);
        let (payload_tx, payload_rx) = mpsc::sync_channel::<ConnectionRestoreRuntime>(1);
        let event_tx = events.clone();
        let worker = spawn(Box::new(move || {
            if let Ok(restored) = payload_rx.recv() {
                let outcome = restored.run(
                    token,
                    forwards,
                    attached,
                    worker_cancellation,
                    event_tx.clone(),
                );
                let _ = outcome_tx.send(outcome);
                let _ = event_tx.send(ClientEvent::ConnectionRestoreComplete { token, host_id });
            }
        }));
        let worker = match worker {
            Ok(worker) => worker,
            Err(error) => {
                self.gate.invalidate(host_id);
                runtime.abort_host_connection(host_id);
                fail(
                    state,
                    host_id,
                    format!("SSH connected, but the restoration worker could not start: {error}"),
                );
                return;
            }
        };
        let restored = match runtime.split_connection_restore(host_id, token, cancellation.clone())
        {
            Ok(restored) => restored,
            Err(error) => {
                self.gate.invalidate(host_id);
                drop(payload_tx);
                let _ = worker.join();
                runtime.abort_host_connection(host_id);
                fail(
                    state,
                    host_id,
                    format!("SSH connected, but background restoration could not start: {error}"),
                );
                return;
            }
        };
        if let Err(error) = payload_tx.send(restored) {
            self.gate.invalidate(host_id);
            drop(error.0);
            runtime.forget_connection_restore(host_id, token);
            let _ = worker.join();
            fail(
                state,
                host_id,
                "SSH restoration worker exited before accepting the connection; reconnect the host."
                    .to_owned(),
            );
            return;
        }
        self.attach_test_job(host_id, generation, token, cancellation, outcome_rx, worker);
    }
}

fn fail(state: &mut ClientState, host_id: HostId, message: String) {
    state.connections.insert(host_id, HostConnection::Failed);
    state.set_output(message);
}
