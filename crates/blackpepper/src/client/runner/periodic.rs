//! Coalesced host polling that never waits on the terminal/render thread.

mod apply;
#[cfg(test)]
mod tests;

use super::super::runtime::{
    ClientRuntime, ForwardCleanupBatch, ForwardCleanupOutcome, PeriodicRefreshJob,
};
use super::super::{ClientEvent, ClientState, HostConnection};
use crate::core::{HostId, HostPeriodicRefresh};
use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, Sender};

struct InFlightRefresh {
    token: uuid::Uuid,
    valid: bool,
    cleanup_forward_ids: Vec<uuid::Uuid>,
    cancellation: Option<Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

#[derive(Default)]
pub(super) struct Coordinator {
    in_flight: BTreeMap<HostId, InFlightRefresh>,
}

impl Coordinator {
    fn begin(
        &mut self,
        host_id: HostId,
        cleanup_forward_ids: Vec<uuid::Uuid>,
    ) -> Option<uuid::Uuid> {
        if self.in_flight.contains_key(&host_id) {
            return None;
        }
        let token = uuid::Uuid::new_v4();
        self.in_flight.insert(
            host_id,
            InFlightRefresh {
                token,
                valid: true,
                cleanup_forward_ids,
                cancellation: None,
                worker: None,
            },
        );
        Some(token)
    }

    fn attach_worker(
        &mut self,
        host_id: HostId,
        token: uuid::Uuid,
        cancellation: Sender<()>,
        worker: std::thread::JoinHandle<()>,
    ) {
        if let Some(refresh) = self
            .in_flight
            .get_mut(&host_id)
            .filter(|refresh| refresh.token == token)
        {
            refresh.cancellation = Some(cancellation);
            refresh.worker = Some(worker);
        } else {
            let _ = cancellation.send(());
            let _ = worker.join();
        }
    }

    /// Keep the bounded worker accounted for, but reject its result after the
    /// underlying SSH connection changes generation.
    pub(super) fn invalidate(&mut self, host_id: HostId) -> Vec<uuid::Uuid> {
        if let Some(refresh) = self.in_flight.get_mut(&host_id) {
            refresh.valid = false;
            if let Some(cancellation) = refresh.cancellation.take() {
                let _ = cancellation.send(());
            }
            return std::mem::take(&mut refresh.cleanup_forward_ids);
        }
        Vec::new()
    }

    fn finish(&mut self, host_id: HostId, token: uuid::Uuid) -> bool {
        let valid = self
            .in_flight
            .get(&host_id)
            .is_some_and(|refresh| refresh.token == token && refresh.valid);
        if self
            .in_flight
            .get(&host_id)
            .is_some_and(|refresh| refresh.token == token)
        {
            let mut refresh = self
                .in_flight
                .remove(&host_id)
                .expect("matching in-flight refresh disappeared");
            if let Some(worker) = refresh.worker.take() {
                let _ = worker.join();
            }
        }
        valid
    }

    pub(super) fn shutdown(&mut self) {
        let mut refreshes = std::mem::take(&mut self.in_flight)
            .into_values()
            .collect::<Vec<_>>();
        for refresh in &mut refreshes {
            if let Some(cancellation) = refresh.cancellation.take() {
                let _ = cancellation.send(());
            }
        }
        for mut refresh in refreshes {
            if let Some(worker) = refresh.worker.take() {
                let _ = worker.join();
            }
        }
    }
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(super) fn schedule(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    periodic: &mut Coordinator,
    sender: &Sender<ClientEvent>,
) {
    let host_ids = state
        .connections
        .iter()
        .filter_map(|(host_id, connection)| {
            (matches!(
                connection,
                HostConnection::Local | HostConnection::Connected
            ) && !runtime.host_is_owned_by_background_work(*host_id))
            .then_some(*host_id)
        })
        .collect::<Vec<_>>();
    for host_id in host_ids {
        let Some(token) = periodic.begin(host_id, Vec::new()) else {
            continue;
        };
        let attached = state
            .terminals
            .keys()
            .copied()
            .filter(|workspace_id| state.host_for_workspace(*workspace_id) == Some(host_id))
            .collect::<Vec<_>>();
        match runtime.start_periodic_refresh(host_id, attached) {
            Ok(job) => {
                let (cancellation, worker) = spawn_periodic_task(token, job, sender.clone());
                periodic.attach_worker(host_id, token, cancellation, worker);
            }
            Err(error) => {
                periodic.finish(host_id, token);
                let _ = apply::refresh(state, runtime, host_id, Err(error));
            }
        }
    }
}

pub(super) fn complete(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    periodic: &mut Coordinator,
    token: uuid::Uuid,
    host_id: HostId,
    result: Result<Box<HostPeriodicRefresh>, String>,
) {
    if runtime.host_is_owned_by_background_work(host_id) {
        invalidate_host(state, periodic, host_id);
    }
    let valid = periodic.finish(host_id, token);
    if !valid {
        return;
    }
    if let Some(cleanup) = apply::refresh(state, runtime, host_id, result) {
        start_forward_cleanup(state, periodic, cleanup);
    }
}

pub(super) fn complete_forward_cleanup(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    periodic: &mut Coordinator,
    token: uuid::Uuid,
    host_id: HostId,
    outcomes: Vec<ForwardCleanupOutcome>,
) {
    if runtime.host_is_owned_by_background_work(host_id) {
        invalidate_host(state, periodic, host_id);
    }
    if !periodic.finish(host_id, token) {
        return;
    }
    apply::forward_cleanup(state, runtime, host_id, outcomes);
}

/// An observation started before an explicit mutation cannot be merged after
/// that mutation returns. Invalidating at ownership acquisition closes the
/// gap that a completion-time busy check alone cannot cover.
pub(super) fn invalidate_owned(
    state: &mut ClientState,
    runtime: &ClientRuntime,
    periodic: &mut Coordinator,
) {
    for host_id in runtime.background_owned_host_ids() {
        invalidate_host(state, periodic, host_id);
    }
}

/// A cancelled cleanup has an unknown outcome: its exact forward may still be
/// live, while applying its stale result after a host mutation is unsafe. Make
/// only the generation's original targets retryable; replacement forwards and
/// unrelated state remain untouched.
pub(super) fn invalidate_host(
    state: &mut ClientState,
    periodic: &mut Coordinator,
    host_id: HostId,
) {
    let retryable = periodic.invalidate(host_id);
    for forward in state.forwards.iter_mut().filter(|forward| {
        forward.host_id == host_id
            && retryable.contains(&forward.id)
            && forward.status == crate::ports::ForwardStatus::Cancelling
    }) {
        forward.status = crate::ports::ForwardStatus::Failed(
            "Background tunnel cleanup was interrupted by newer host activity; it will be retried after that activity finishes."
                .to_owned(),
        );
    }
}

pub(super) fn apply_connection_refresh(
    state: &mut ClientState,
    host_id: HostId,
    refresh: &HostPeriodicRefresh,
) -> Vec<String> {
    apply::merge_refresh_state(state, host_id, refresh)
}

pub(super) fn mark_connection_refresh_failed(
    state: &mut ClientState,
    host_id: HostId,
    error: &str,
) {
    apply::mark_host_agents_failed(state, host_id, error);
}

fn spawn_periodic_task(
    token: uuid::Uuid,
    job: PeriodicRefreshJob,
    sender: Sender<ClientEvent>,
) -> (Sender<()>, std::thread::JoinHandle<()>) {
    let host_id = job.host_id;
    spawn_refresh_waiter(
        token,
        host_id,
        move |cancellation| job.wait_cancellable(cancellation),
        sender,
    )
}

fn spawn_refresh_waiter(
    token: uuid::Uuid,
    host_id: HostId,
    wait: impl FnOnce(Receiver<()>) -> Result<HostPeriodicRefresh, String> + Send + 'static,
    sender: Sender<ClientEvent>,
) -> (Sender<()>, std::thread::JoinHandle<()>) {
    let (cancellation, cancelled) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = wait(cancelled).map(Box::new);
        let _ = sender.send(ClientEvent::PeriodicRefreshComplete {
            token,
            host_id,
            result,
        });
    });
    (cancellation, worker)
}

fn start_forward_cleanup(
    state: &mut ClientState,
    periodic: &mut Coordinator,
    cleanup: ForwardCleanupBatch,
) {
    let host_id = cleanup.host_id;
    let cleanup_forward_ids = cleanup.forward_ids().collect();
    let Some(token) = periodic.begin(host_id, cleanup_forward_ids) else {
        return;
    };
    let (cancellation, cancelled) = mpsc::channel();
    let sender = state.event_tx.clone();
    let worker = std::thread::spawn(move || {
        let outcomes = cleanup.wait_cancellable(cancelled);
        let _ = sender.send(ClientEvent::PeriodicForwardCleanupComplete {
            token,
            host_id,
            outcomes,
        });
    });
    periodic.attach_worker(host_id, token, cancellation, worker);
}
