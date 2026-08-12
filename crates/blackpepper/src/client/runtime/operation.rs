//! Host-scoped explicit work moved off the terminal/render thread.

mod owned;
mod types;

use super::ClientRuntime;
use crate::client::ClientEvent;
use crate::core::HostId;
use crate::transport::CommandCancellation;
use owned::HostOperationRuntime;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use types::{CompletedHostOperation, HostOperationWork};
pub(crate) use types::{
    DeferredHostAction, DeferredHostResult, HostOperationContext, HostOperationValue,
    WorktreeMutationResult,
};

type WorkerOutcome = (
    HostOperationRuntime,
    Result<HostOperationValue, String>,
    Result<crate::core::RegistrySnapshot, String>,
    Vec<DeferredHostResult>,
);

pub(crate) enum DurableActionQueue {
    Started { token: uuid::Uuid, label: String },
    Queued { behind: String },
}

pub(super) struct ActiveHostOperation {
    generation: u64,
    token: uuid::Uuid,
    label: String,
    context: Option<HostOperationContext>,
    cancellation: CommandCancellation,
    discard_host: bool,
    discard_signal: Arc<AtomicBool>,
    deferred_seen: Arc<AtomicBool>,
    outcome: Receiver<WorkerOutcome>,
    worker: Option<JoinHandle<()>>,
    deferred: Arc<Mutex<Vec<DeferredHostAction>>>,
}

impl Drop for ActiveHostOperation {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(worker) = self.worker.take() {
            // Every production operation waits through CommandCancellation,
            // but do not let an accidental non-cooperative Rust callback make
            // application quit hang forever. A detached worker loses its
            // outcome receiver and drops its owned transport when it returns.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while !worker.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
        drop(self.outcome.try_recv());
    }
}

impl ClientRuntime {
    pub(crate) fn start_host_operation(
        &mut self,
        host_id: HostId,
        label: impl Into<String>,
        context: HostOperationContext,
        events: Sender<ClientEvent>,
        work: HostOperationWork,
    ) -> Result<uuid::Uuid, String> {
        if let Some(label) = self.host_operation_label(host_id) {
            return Err(format!(
                "This host is already busy with {label}; wait for it to finish or press Esc to cancel it."
            ));
        }
        if self.connection_restores.contains_key(&host_id) {
            return Err(
                "This host is still being restored; wait for SSH recovery to finish.".into(),
            );
        }

        let generation = {
            let generation = self.host_operation_generations.entry(host_id).or_default();
            *generation = generation.saturating_add(1);
            *generation
        };
        let token = uuid::Uuid::new_v4();
        let label = label.into();
        let cancellation = CommandCancellation::default();
        let worker_cancellation = cancellation.clone();
        let (payload_tx, payload_rx) = mpsc::sync_channel::<HostOperationRuntime>(1);
        let (outcome_tx, outcome_rx) = mpsc::sync_channel::<WorkerOutcome>(1);
        let event_tx = events.clone();
        let deferred = Arc::new(Mutex::new(Vec::new()));
        let worker_deferred = Arc::clone(&deferred);
        let discard_signal = Arc::new(AtomicBool::new(false));
        let worker_discard = Arc::clone(&discard_signal);
        let deferred_seen = Arc::new(AtomicBool::new(false));
        let worker_deferred_seen = Arc::clone(&deferred_seen);
        let worker = std::thread::Builder::new()
            .name(format!("bp-operation-{host_id}"))
            .spawn(move || {
                let Ok(mut owned) = payload_rx.recv() else {
                    return;
                };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker_cancellation.scoped(|| match owned.initialize() {
                        Ok(runtime) => work(runtime),
                        Err(error) => Err(error),
                    })
                }))
                .unwrap_or_else(|_| Err("The background operation panicked.".to_owned()));
                let deferred_actions =
                    take_deferred_actions_recording(&worker_deferred, &worker_deferred_seen);
                let deferred_results = if worker_discard.load(Ordering::Acquire) {
                    deferred_actions
                        .into_iter()
                        .map(|action| {
                            failed_action(
                                action,
                                "The host disconnected before this state update could be persisted."
                                    .to_owned(),
                            )
                        })
                        .collect()
                } else {
                    apply_deferred_actions(&mut owned, host_id, deferred_actions)
                };
                let snapshot = owned.snapshot();
                let _ = outcome_tx.send((owned, result, snapshot, deferred_results));
                let _ = event_tx.send(ClientEvent::HostOperationComplete {
                    token,
                    host_id,
                    generation,
                });
            })
            .map_err(|error| format!("Could not start the background operation: {error}"))?;

        let operation_runtime = match self.split_host_operation(host_id) {
            Ok(runtime) => runtime,
            Err(error) => {
                drop(payload_tx);
                let _ = worker.join();
                return Err(error);
            }
        };
        if let Err(error) = payload_tx.send(operation_runtime) {
            let mut runtime = error.0;
            let merge = self.merge_host_operation_runtime(host_id, &mut runtime);
            let _ = worker.join();
            return Err(match merge {
                Ok(()) => "The background operation exited before accepting its host.".into(),
                Err(merge) => format!(
                    "The background operation exited before accepting its host; restoring host ownership also failed: {merge}"
                ),
            });
        }
        self.host_operations.insert(
            host_id,
            ActiveHostOperation {
                generation,
                token,
                label,
                context: Some(context),
                cancellation,
                discard_host: false,
                discard_signal,
                deferred_seen,
                outcome: outcome_rx,
                worker: Some(worker),
                deferred,
            },
        );
        Ok(token)
    }

    pub(crate) fn host_operation_label(&self, host_id: HostId) -> Option<&str> {
        self.host_operations
            .get(&host_id)
            .map(|operation| operation.label.as_str())
    }

    pub(crate) fn host_is_owned_by_background_work(&self, host_id: HostId) -> bool {
        self.host_operations.contains_key(&host_id)
            || self.connection_restores.contains_key(&host_id)
    }

    pub(crate) fn background_owned_host_ids(&self) -> Vec<HostId> {
        self.host_operations
            .keys()
            .chain(self.connection_restores.keys())
            .copied()
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn host_operation_active(&self, host_id: HostId) -> bool {
        self.host_operations.contains_key(&host_id)
    }

    pub(crate) fn cancel_host_operation(&mut self, host_id: HostId) -> Option<String> {
        let operation = self.host_operations.get(&host_id)?;
        operation.cancellation.cancel();
        Some(operation.label.clone())
    }

    pub(crate) fn finish_host_operation(
        &mut self,
        host_id: HostId,
        generation: u64,
        token: uuid::Uuid,
    ) -> Option<CompletedHostOperation> {
        let current = self.host_operations.get(&host_id).is_some_and(|operation| {
            operation.generation == generation && operation.token == token
        });
        if !current {
            return None;
        }
        let mut operation = self.host_operations.remove(&host_id)?;
        if let Some(worker) = operation.worker.take() {
            let _ = worker.join();
        }
        let (mut runtime, mut result, snapshot, deferred_results) = match operation.outcome.recv() {
            Ok(outcome) => outcome,
            Err(_) => {
                return Some(CompletedHostOperation {
                    host_id,
                    label: operation.label.clone(),
                    context: operation.context.take()?,
                    result: Err("The background operation exited without a result.".into()),
                    snapshot: Err(
                        "The background operation exited before taking a registry snapshot.".into(),
                    ),
                    deferred_results: Vec::new(),
                    deferred_remaining: take_deferred_actions(&operation.deferred),
                    discarded: operation.discard_host,
                });
            }
        };
        if operation.discard_host {
            drop(runtime);
            return Some(CompletedHostOperation {
                host_id,
                label: operation.label.clone(),
                context: operation.context.take()?,
                result,
                snapshot,
                deferred_results,
                deferred_remaining: take_deferred_actions(&operation.deferred),
                discarded: true,
            });
        }
        if let Err(error) = self.merge_host_operation_runtime(host_id, &mut runtime) {
            result = Err(format!(
                "{} Host ownership could not be restored: {error}",
                result
                    .err()
                    .map(|error| format!("{error}."))
                    .unwrap_or_default()
            ));
        }
        Some(CompletedHostOperation {
            host_id,
            label: operation.label.clone(),
            context: operation.context.take()?,
            result,
            snapshot,
            deferred_results,
            deferred_remaining: take_deferred_actions(&operation.deferred),
            discarded: false,
        })
    }

    #[cfg(test)]
    fn test_operation_identity(&self, host_id: HostId) -> Option<(u64, uuid::Uuid)> {
        self.host_operations
            .get(&host_id)
            .map(|operation| (operation.generation, operation.token))
    }

    pub(super) fn cancel_host_operation_for_disconnect(&mut self, host_id: HostId) {
        let deferred = if let Some(operation) = self.host_operations.get_mut(&host_id) {
            operation.discard_host = true;
            operation.discard_signal.store(true, Ordering::Release);
            operation.cancellation.cancel();
            take_deferred_actions(&operation.deferred)
        } else {
            Vec::new()
        };
        self.deferred_host_actions
            .entry(host_id)
            .or_default()
            .extend(deferred);
    }

    pub(super) fn disconnect_operation_warning(&self, host_id: HostId) -> Option<String> {
        let operation = self.host_operations.get(&host_id)?;
        let worktrunk = operation.context.as_ref().is_some_and(|context| {
            matches!(context, HostOperationContext::WorktreeMutation { .. })
        });
        let deferred = operation
            .deferred
            .lock()
            .map(|actions| !actions.is_empty() || operation.deferred_seen.load(Ordering::Acquire))
            .unwrap_or(true);
        match (worktrunk, deferred) {
            (true, true) => Some(
                "Worktrunk result is Unknown after disconnect; Blackpepper will not retry it. Reconnect and run :worktree list to reconcile. Queued terminal status may not have persisted; it remains visibly Unknown in this client."
                    .to_owned(),
            ),
            (true, false) => Some(
                "Worktrunk result is Unknown after disconnect; Blackpepper will not retry it. Reconnect and run :worktree list to reconcile."
                    .to_owned(),
            ),
            (false, true) => Some(
                "Queued terminal status may not have persisted before disconnect; it remains visibly Unknown in this client."
                    .to_owned(),
            ),
            (false, false) => None,
        }
    }

    pub(crate) fn queue_durable_actions(
        &mut self,
        host_id: HostId,
        label: impl Into<String>,
        actions: Vec<DeferredHostAction>,
        events: Sender<ClientEvent>,
    ) -> Result<DurableActionQueue, String> {
        if actions.is_empty() {
            return Err("No durable host state update was requested.".to_owned());
        }
        if let Some(operation) = self.host_operations.get(&host_id) {
            operation
                .deferred
                .lock()
                .map_err(|_| "The pending host-state queue was poisoned.".to_owned())?
                .extend(actions);
            return Ok(DurableActionQueue::Queued {
                behind: operation.label.clone(),
            });
        }
        if self.connection_restores.contains_key(&host_id) {
            self.deferred_host_actions
                .entry(host_id)
                .or_default()
                .extend(actions);
            return Ok(DurableActionQueue::Queued {
                behind: "SSH recovery".to_owned(),
            });
        }
        let label = label.into();
        let work_actions = actions;
        let token = self.start_host_operation(
            host_id,
            label.clone(),
            HostOperationContext::DurableState,
            events,
            Box::new(move |runtime| {
                Ok(HostOperationValue::DurableState(apply_actions(
                    runtime,
                    host_id,
                    work_actions,
                )))
            }),
        )?;
        Ok(DurableActionQueue::Started { token, label })
    }

    pub(crate) fn start_queued_durable_actions(
        &mut self,
        host_id: HostId,
        events: Sender<ClientEvent>,
    ) -> Result<Option<(uuid::Uuid, String)>, String> {
        let Some(actions) = self.deferred_host_actions.remove(&host_id) else {
            return Ok(None);
        };
        match self.queue_durable_actions(
            host_id,
            "Persisting queued terminal state",
            actions.clone(),
            events,
        ) {
            Ok(DurableActionQueue::Started { token, label }) => Ok(Some((token, label))),
            Ok(DurableActionQueue::Queued { .. }) => Ok(None),
            Err(error) => {
                self.deferred_host_actions.insert(host_id, actions);
                Err(error)
            }
        }
    }

    pub(crate) fn fail_queued_durable_actions(
        &mut self,
        host_id: HostId,
        message: &str,
    ) -> Vec<DeferredHostResult> {
        self.deferred_host_actions
            .remove(&host_id)
            .unwrap_or_default()
            .into_iter()
            .map(|action| failed_action(action, message.to_owned()))
            .collect()
    }

    /// Shutdown does not need operation results, but it must cancel every
    /// owned child before terminal teardown. Removing entries invokes the
    /// bounded ActiveHostOperation drop path.
    pub(crate) fn shutdown_host_operations(&mut self) {
        self.host_operations.clear();
    }
}

fn take_deferred_actions(
    deferred: &Arc<Mutex<Vec<DeferredHostAction>>>,
) -> Vec<DeferredHostAction> {
    deferred
        .lock()
        .map(|mut actions| std::mem::take(&mut *actions))
        .unwrap_or_default()
}

fn take_deferred_actions_recording(
    deferred: &Arc<Mutex<Vec<DeferredHostAction>>>,
    seen: &AtomicBool,
) -> Vec<DeferredHostAction> {
    deferred
        .lock()
        .map(|mut actions| {
            if !actions.is_empty() {
                seen.store(true, Ordering::Release);
            }
            std::mem::take(&mut *actions)
        })
        .unwrap_or_default()
}

fn apply_deferred_actions(
    owned: &mut HostOperationRuntime,
    host_id: HostId,
    actions: Vec<DeferredHostAction>,
) -> Vec<DeferredHostResult> {
    if actions.is_empty() {
        return Vec::new();
    }
    match owned.initialize() {
        Ok(runtime) => {
            CommandCancellation::mask_current(|| apply_actions(runtime, host_id, actions))
        }
        Err(error) => actions
            .into_iter()
            .map(|action| failed_action(action, error.clone()))
            .collect(),
    }
}

pub(super) fn apply_actions(
    runtime: &mut ClientRuntime,
    host_id: HostId,
    actions: Vec<DeferredHostAction>,
) -> Vec<DeferredHostResult> {
    actions
        .into_iter()
        .map(|action| match action {
            DeferredHostAction::MarkDetached { workspace_id } => DeferredHostResult::Detached {
                workspace_id,
                result: runtime.mark_detached(workspace_id),
            },
            DeferredHostAction::MarkAgentsUnknown {
                workspace_id,
                run_ids,
            } => DeferredHostResult::AgentsUnknown {
                workspace_id,
                results: run_ids
                    .into_iter()
                    .map(|run_id| (run_id, runtime.mark_agent_state_unknown(host_id, run_id)))
                    .collect(),
            },
        })
        .collect()
}

fn failed_action(action: DeferredHostAction, error: String) -> DeferredHostResult {
    match action {
        DeferredHostAction::MarkDetached { workspace_id } => DeferredHostResult::Detached {
            workspace_id,
            result: Err(error),
        },
        DeferredHostAction::MarkAgentsUnknown {
            workspace_id,
            run_ids,
        } => DeferredHostResult::AgentsUnknown {
            workspace_id,
            results: run_ids
                .into_iter()
                .map(|run_id| (run_id, Err(error.clone())))
                .collect(),
        },
    }
}

#[cfg(test)]
mod tests;
