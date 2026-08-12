//! Generation-checked SSH recovery that never owns the render thread.

mod generation;
mod start;

use super::super::runtime::{
    ClientRuntime, ConnectionRestoreReport, ConnectionRestoreRuntime, ConnectionUpdate,
};
use super::super::{ClientState, HostConnection};
use super::connection_update;
use crate::core::HostId;
use crate::transport::CommandCancellation;
use std::collections::BTreeMap;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;

use generation::GenerationGate;

type RestoreOutcome = (ConnectionRestoreRuntime, ConnectionRestoreReport);
pub(super) type RestoreWorker = Box<dyn FnOnce() + Send + 'static>;

struct InFlightRestore {
    host_id: HostId,
    generation: u64,
    token: uuid::Uuid,
    valid: bool,
    cancellation: CommandCancellation,
    outcome: Receiver<RestoreOutcome>,
    worker: Option<JoinHandle<()>>,
}

#[derive(Default)]
pub(super) struct Coordinator {
    gate: GenerationGate,
    jobs: BTreeMap<uuid::Uuid, InFlightRestore>,
}

impl Coordinator {
    fn attach_test_job(
        &mut self,
        host_id: HostId,
        generation: u64,
        token: uuid::Uuid,
        cancellation: CommandCancellation,
        outcome: Receiver<RestoreOutcome>,
        worker: JoinHandle<()>,
    ) {
        self.jobs.insert(
            token,
            InFlightRestore {
                host_id,
                generation,
                token,
                valid: true,
                cancellation,
                outcome,
                worker: Some(worker),
            },
        );
    }

    pub(super) fn progress(
        &self,
        state: &mut ClientState,
        token: uuid::Uuid,
        host_id: HostId,
        message: String,
    ) {
        let Some(job) = self.jobs.get(&token) else {
            return;
        };
        if job.valid && self.gate.is_current(host_id, job.generation, token) {
            state.set_output(format!("SSH recovery: {message}"));
        }
    }

    pub(super) fn complete(
        &mut self,
        state: &mut ClientState,
        runtime: &mut ClientRuntime,
        token: uuid::Uuid,
        host_id: HostId,
    ) {
        let Some(mut job) = self.jobs.remove(&token) else {
            return;
        };
        if let Some(worker) = job.worker.take() {
            let _ = worker.join();
        }
        let Ok((restored, mut report)) = job.outcome.recv() else {
            self.gate.invalidate(host_id);
            runtime.forget_connection_restore(host_id, token);
            state.connections.insert(host_id, HostConnection::Failed);
            state.set_output("SSH restoration worker exited without a result; reconnect the host.");
            super::operations::apply_deferred_results(
                state,
                runtime.fail_queued_durable_actions(
                    host_id,
                    "SSH restoration exited before durable terminal state could be written.",
                ),
            );
            return;
        };
        let current = job.valid
            && self.gate.finish(host_id, job.generation, job.token)
            && state.connections.get(&host_id) == Some(&HostConnection::Reconnecting);
        if !current {
            drop(restored);
            runtime.forget_connection_restore(host_id, token);
            return;
        }
        if let Some(error) = &report.connection_error {
            drop(restored);
            runtime.forget_connection_restore(host_id, token);
            connection_update::apply(
                state,
                ConnectionUpdate::Failed {
                    host_id,
                    message: format!("SSH disconnected during restoration: {error}"),
                },
            );
            super::operations::apply_deferred_results(
                state,
                runtime.fail_queued_durable_actions(
                    host_id,
                    "SSH disconnected before durable terminal state could be written.",
                ),
            );
            return;
        }
        if report.cancelled {
            drop(restored);
            runtime.forget_connection_restore(host_id, token);
            connection_update::apply(
                state,
                ConnectionUpdate::Failed {
                    host_id,
                    message: "SSH restoration was cancelled; reconnect the host.".to_owned(),
                },
            );
            super::operations::apply_deferred_results(
                state,
                runtime.fail_queued_durable_actions(
                    host_id,
                    "SSH recovery was cancelled before durable terminal state could be written.",
                ),
            );
            return;
        }
        let previous_host_id = report.previous_host_id;
        if let Err(error) = runtime.merge_connection_restore(restored) {
            runtime.forget_connection_restore(previous_host_id, token);
            state.connections.insert(host_id, HostConnection::Failed);
            state.set_output(error);
            super::operations::apply_deferred_results(
                state,
                runtime.fail_queued_durable_actions(
                    host_id,
                    "SSH ownership could not be restored for the durable state update.",
                ),
            );
            return;
        }
        state.connections.remove(&report.previous_host_id);
        state
            .connections
            .insert(report.host_id, HostConnection::Connected);
        state.selected_host = Some(report.host_id);
        if let Some(refresh) = &report.refresh {
            report
                .watcher_errors
                .extend(runtime.ensure_periodic_blocker_watchers(refresh, state.event_tx.clone()));
        }
        connection_update::apply_restored(state, report);
        match runtime.start_queued_durable_actions(host_id, state.event_tx.clone()) {
            Ok(Some((token, label))) => {
                state.host_operations.insert(host_id, (token, label));
            }
            Ok(None) => {}
            Err(error) => super::operations::apply_deferred_results(
                state,
                runtime.fail_queued_durable_actions(host_id, &error),
            ),
        }
    }

    pub(super) fn invalidate(&mut self, host_id: HostId) {
        self.gate.invalidate(host_id);
        for job in self.jobs.values_mut().filter(|job| job.host_id == host_id) {
            job.valid = false;
            job.cancellation.cancel();
        }
    }

    pub(super) fn cancel_disconnected(&mut self, state: &ClientState) {
        let hosts = disconnected_hosts(
            &self.jobs,
            |host_id| state.connections.get(&host_id) == Some(&HostConnection::Reconnecting),
            |_, _| false,
        );
        for host_id in hosts {
            self.invalidate(host_id);
        }
    }

    /// A host command can resolve the synchronized stable ID while this
    /// coordinator is still keyed by the temporary connection ID. Treat any
    /// disconnected destination as invalidating every restore generation for
    /// that destination before a queued completion can merge it.
    pub(super) fn reconcile_user_disconnects(
        &mut self,
        state: &ClientState,
        runtime: &ClientRuntime,
    ) {
        let hosts = disconnected_hosts(
            &self.jobs,
            |host_id| state.connections.get(&host_id) == Some(&HostConnection::Reconnecting),
            |host_id, token| runtime.connection_restore_cancelled(host_id, token),
        );
        for host_id in hosts {
            self.invalidate(host_id);
        }
    }

    pub(super) fn shutdown(&mut self) {
        self.gate.active.clear();
        let mut jobs = std::mem::take(&mut self.jobs)
            .into_values()
            .collect::<Vec<_>>();
        for job in &mut jobs {
            job.valid = false;
            job.cancellation.cancel();
        }
        for mut job in jobs {
            if let Some(worker) = job.worker.take() {
                let _ = worker.join();
            }
            drop(job.outcome.try_recv());
        }
    }
}

fn disconnected_hosts(
    jobs: &BTreeMap<uuid::Uuid, InFlightRestore>,
    state_is_restoring: impl Fn(HostId) -> bool,
    runtime_cancelled: impl Fn(HostId, uuid::Uuid) -> bool,
) -> Vec<HostId> {
    jobs.values()
        .filter_map(|job| {
            (!state_is_restoring(job.host_id) || runtime_cancelled(job.host_id, job.token))
                .then_some(job.host_id)
        })
        .collect()
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests;
