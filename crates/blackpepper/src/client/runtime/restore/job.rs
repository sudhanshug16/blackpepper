use super::{ConnectionRestoreReport, ConnectionRestoreRuntime};
use crate::client::runtime::{connection, HostSlot};
use crate::client::ClientEvent;
use crate::core::{HostId, HostPeriodicRefresh, WorkspaceId};
use crate::ports::ForwardState;
use crate::transport::{CommandCancellation, ConnectionState};
use std::sync::mpsc::Sender;

impl ConnectionRestoreRuntime {
    pub(crate) fn run(
        mut self,
        token: uuid::Uuid,
        mut forwards: Vec<ForwardState>,
        attached_workspaces: Vec<WorkspaceId>,
        cancellation: CommandCancellation,
        events: Sender<ClientEvent>,
    ) -> (Self, ConnectionRestoreReport) {
        let connection_id = self.connection_id;
        let report = cancellation.scoped(|| {
            if let Err(error) = self.initialize(&cancellation) {
                return self.failed_report(
                    forwards,
                    cancellation.is_cancelled(),
                    format!("Restore registry initialization failed: {error}"),
                );
            }
            progress(
                &events,
                token,
                connection_id,
                "Synchronizing the remote registry…",
            );
            if let Err(error) = self.synchronize_registry() {
                return self.failed_report(
                    forwards,
                    cancellation.is_cancelled(),
                    format!("SSH registry synchronization failed: {error}"),
                );
            }
            let host_id = self.host_id;
            for forward in &mut forwards {
                if forward.host_id == connection_id {
                    forward.host_id = host_id;
                }
            }
            progress(
                &events,
                token,
                connection_id,
                "Checking registry and tunnels…",
            );
            let mut errors = Vec::new();
            match self.runtime_mut().snapshot() {
                Ok(snapshot) => {
                    let cleanup = self
                        .runtime_mut()
                        .reconcile_forwards(&mut forwards, &snapshot);
                    errors.extend(cleanup.failures);
                }
                Err(error) => errors.push(format!("Initial registry snapshot: {error}")),
            }
            let restored_workspaces = self.restore_workspaces(
                token,
                connection_id,
                host_id,
                &cancellation,
                &events,
                &mut errors,
            );
            let (refresh, watcher_errors) = self.restore_observations(
                token,
                connection_id,
                host_id,
                attached_workspaces,
                &cancellation,
                &events,
                &mut errors,
            );
            if !cancellation.is_cancelled() {
                progress(
                    &events,
                    token,
                    connection_id,
                    "Restoring client-owned tunnels…",
                );
                for forward in &mut forwards {
                    if cancellation.is_cancelled() {
                        break;
                    }
                    self.runtime_mut().reconnect_forward(forward);
                }
            }
            let snapshot = if cancellation.is_cancelled() {
                Err("Connection restoration was cancelled.".to_owned())
            } else {
                self.runtime_mut().snapshot()
            };
            let connection_error = self.connection_error();
            ConnectionRestoreReport {
                previous_host_id: connection_id,
                host_id,
                snapshot,
                refresh,
                forwards,
                errors,
                restored_workspaces,
                watcher_errors,
                cancelled: cancellation.is_cancelled(),
                connection_error,
            }
        });
        if report.cancelled {
            self.abort();
        }
        (self, report)
    }

    fn synchronize_registry(&mut self) -> Result<(), String> {
        let previous = self.host_id;
        let reserved_host_ids = self.reserved_host_ids.clone();
        let host_id = connection::synchronize_registry_with_reserved(
            self.runtime_mut(),
            previous,
            &reserved_host_ids,
        )?;
        self.host_id = host_id;
        if previous != host_id {
            if let Some(pending) = self
                .runtime_mut()
                .remote_pending_worktree_removals
                .remove(&previous)
            {
                self.runtime_mut()
                    .remote_pending_worktree_removals
                    .insert(host_id, pending);
            }
            for watcher in self.runtime_mut().blocker_watchers.values_mut() {
                if watcher.host_id == previous {
                    watcher.host_id = host_id;
                }
            }
        }
        Ok(())
    }

    fn restore_workspaces(
        &mut self,
        token: uuid::Uuid,
        connection_id: HostId,
        host_id: HostId,
        cancellation: &CommandCancellation,
        events: &Sender<ClientEvent>,
        errors: &mut Vec<String>,
    ) -> Option<usize> {
        if cancellation.is_cancelled() {
            return None;
        }
        progress(
            events,
            token,
            connection_id,
            "Restoring registered workspace shells…",
        );
        match self.runtime_mut().restore_host_workspaces_with(
            host_id,
            || cancellation.is_cancelled(),
            |index, total| {
                progress(
                    events,
                    token,
                    connection_id,
                    &format!("Restoring workspace {index}/{total}…"),
                );
            },
        ) {
            Ok(count) => Some(count),
            Err(error) => {
                if !cancellation.is_cancelled() {
                    errors.push(error);
                }
                None
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn restore_observations(
        &mut self,
        token: uuid::Uuid,
        connection_id: HostId,
        host_id: HostId,
        attached_workspaces: Vec<WorkspaceId>,
        cancellation: &CommandCancellation,
        events: &Sender<ClientEvent>,
        errors: &mut Vec<String>,
    ) -> (Option<HostPeriodicRefresh>, Vec<String>) {
        if cancellation.is_cancelled() {
            return (None, Vec::new());
        }
        progress(
            events,
            token,
            connection_id,
            "Recovering agent status and clients…",
        );
        match self
            .runtime_mut()
            .start_periodic_refresh(host_id, attached_workspaces)
            .and_then(|job| job.wait_with_token(cancellation))
        {
            Ok(observed) => match self.runtime_mut().apply_periodic_registry(&observed) {
                // The completed runtime must be merged before watcher reader
                // events can be accepted as current by the UI runtime.
                Ok(_) => {
                    self.runtime_mut()
                        .prune_periodic_blocker_watchers(&observed);
                    (Some(observed), Vec::new())
                }
                Err(error) => {
                    errors.push(format!("Registry recovery: {error}"));
                    (None, Vec::new())
                }
            },
            Err(error) if !cancellation.is_cancelled() => {
                errors.push(format!("Agent/status recovery: {error}"));
                (None, Vec::new())
            }
            Err(_) => (None, Vec::new()),
        }
    }

    fn failed_report(
        &mut self,
        forwards: Vec<ForwardState>,
        cancelled: bool,
        error: String,
    ) -> ConnectionRestoreReport {
        ConnectionRestoreReport {
            previous_host_id: self.connection_id,
            host_id: self.host_id,
            snapshot: Err(error.clone()),
            refresh: None,
            forwards,
            errors: Vec::new(),
            restored_workspaces: None,
            watcher_errors: Vec::new(),
            cancelled,
            connection_error: Some(error),
        }
    }

    fn connection_error(&mut self) -> Option<String> {
        let host_id = self.host_id;
        let Some(HostSlot::Ssh(host)) = self.runtime_mut().hosts.get_mut(&host_id) else {
            return Some("The SSH transport disappeared during restoration.".to_owned());
        };
        match host.transport.poll_connection() {
            Ok(ConnectionState::Ready) => None,
            Ok(state) => Some(format!(
                "SSH left the ready state during restoration: {state:?}."
            )),
            Err(error) => Some(error.to_string()),
        }
    }
}

fn progress(events: &Sender<ClientEvent>, token: uuid::Uuid, host_id: HostId, message: &str) {
    let _ = events.send(ClientEvent::ConnectionRestoreProgress {
        token,
        host_id,
        message: message.to_owned(),
    });
}
