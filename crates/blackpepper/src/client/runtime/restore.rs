mod job;
mod state;

use super::{ClientRuntime, HostSlot};
use crate::core::{HostId, HostPeriodicRefresh, HostRecord, RegistrySnapshot};
use crate::ports::ForwardState;
use crate::transport::CommandCancellation;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

pub(super) struct ActiveConnectionRestore {
    token: uuid::Uuid,
    destination: String,
    cancellation: CommandCancellation,
}

/// One host-scoped runtime moved off the render thread during first-connect
/// synchronization and reconnect recovery. The main runtime retains the
/// process singleton and every unrelated host.
pub(crate) struct ConnectionRestoreRuntime {
    connection_id: HostId,
    host_id: HostId,
    token: uuid::Uuid,
    reserved_host_ids: BTreeSet<HostId>,
    pending: Option<PendingRestoreRuntime>,
    runtime: Option<ClientRuntime>,
}

pub(super) struct PendingRestoreRuntime {
    paths: crate::core::CorePaths,
    local_host_id: HostId,
    slot: HostSlot,
    helper: Option<String>,
    removals: Option<BTreeSet<crate::core::WorkspaceId>>,
    watchers: BTreeMap<crate::core::AgentRunId, super::blockers::BlockerWatcher>,
}

pub(crate) struct ConnectionRestoreReport {
    pub previous_host_id: HostId,
    pub host_id: HostId,
    pub snapshot: Result<RegistrySnapshot, String>,
    pub refresh: Option<HostPeriodicRefresh>,
    pub forwards: Vec<ForwardState>,
    pub errors: Vec<String>,
    pub restored_workspaces: Option<usize>,
    pub watcher_errors: Vec<String>,
    pub cancelled: bool,
    pub connection_error: Option<String>,
}

impl ClientRuntime {
    pub(crate) fn split_connection_restore(
        &mut self,
        host_id: HostId,
        token: uuid::Uuid,
        cancellation: CommandCancellation,
    ) -> Result<ConnectionRestoreRuntime, String> {
        if host_id == self.local_host_id {
            return Err("The local host does not need SSH connection restoration.".to_owned());
        }
        let destination = match self.hosts.get(&host_id) {
            Some(HostSlot::Ssh(host)) => host.transport.config().destination.clone(),
            _ => return Err("Only a connected SSH host can enter restoration.".to_owned()),
        };
        if self
            .connection_restores
            .values()
            .any(|restore| restore.destination == destination || restore.token == token)
        {
            return Err("This SSH destination already has a restoration worker.".to_owned());
        }
        let reserved_host_ids = self
            .hosts
            .keys()
            .copied()
            .filter(|candidate| *candidate != host_id)
            .collect();
        let slot = self
            .hosts
            .remove(&host_id)
            .ok_or_else(|| format!("Host {host_id} is no longer connected."))?;
        if !matches!(slot, HostSlot::Ssh(_)) {
            self.hosts.insert(host_id, slot);
            return Err("Only SSH hosts can enter connection restoration.".to_owned());
        }
        let helper = self.helper_paths.remove(&host_id);
        let removals = self.remote_pending_worktree_removals.remove(&host_id);
        let (worker_watchers, retained_watchers) = partition_watchers(self, host_id);
        self.blocker_watchers = retained_watchers;
        self.connection_restores.insert(
            host_id,
            ActiveConnectionRestore {
                token,
                destination,
                cancellation: cancellation.clone(),
            },
        );

        Ok(ConnectionRestoreRuntime {
            connection_id: host_id,
            host_id,
            token,
            reserved_host_ids,
            pending: Some(PendingRestoreRuntime {
                paths: self.paths.clone(),
                local_host_id: self.local_host_id,
                slot,
                helper,
                removals,
                watchers: worker_watchers,
            }),
            runtime: None,
        })
    }

    pub(crate) fn merge_connection_restore(
        &mut self,
        mut restored: ConnectionRestoreRuntime,
    ) -> Result<(), String> {
        let current = self
            .connection_restores
            .get(&restored.connection_id)
            .is_some_and(|active| active.token == restored.token);
        if !current {
            return Err("A stale SSH restoration generation was discarded.".to_owned());
        }
        let host_id = restored.host_id;
        if self.hosts.contains_key(&host_id) {
            return Err(format!(
                "A newer SSH connection already owns host {host_id}; the stale restore was discarded."
            ));
        }
        let restored_runtime = restored.runtime.as_mut().ok_or_else(|| {
            "The restore worker returned before opening its registry connection.".to_owned()
        })?;
        let slot = restored_runtime
            .hosts
            .remove(&host_id)
            .ok_or_else(|| "The restore worker returned without its SSH transport.".to_owned())?;
        self.hosts.insert(host_id, slot);
        move_host_entries(restored_runtime, self, host_id);
        self.connection_restores.remove(&restored.connection_id);
        Ok(())
    }

    pub(crate) fn forget_connection_restore(&mut self, host_id: HostId, token: uuid::Uuid) {
        if self
            .connection_restores
            .get(&host_id)
            .is_some_and(|active| active.token == token)
        {
            self.connection_restores.remove(&host_id);
        }
    }

    pub(super) fn connection_restore_matches(&self, host: &HostRecord) -> bool {
        ssh_destination(host).is_some_and(|destination| {
            self.connection_restores
                .values()
                .any(|restore| restore.destination == destination)
        })
    }

    pub(super) fn cancel_connection_restores(&self, host_id: HostId) -> Vec<HostId> {
        let destination = self
            .host_record(host_id)
            .ok()
            .and_then(|record| ssh_destination(&record));
        let mut cancelled = Vec::new();
        for (connection_id, restore) in &self.connection_restores {
            if *connection_id == host_id
                || destination
                    .as_ref()
                    .is_some_and(|value| value == &restore.destination)
            {
                restore.cancellation.cancel();
                cancelled.push(*connection_id);
            }
        }
        cancelled
    }

    pub(crate) fn connection_restore_cancelled(&self, host_id: HostId, token: uuid::Uuid) -> bool {
        self.connection_restores
            .get(&host_id)
            .is_none_or(|restore| restore.token != token || restore.cancellation.is_cancelled())
    }
}

fn partition_watchers(
    runtime: &mut ClientRuntime,
    host_id: HostId,
) -> (
    BTreeMap<crate::core::AgentRunId, super::blockers::BlockerWatcher>,
    BTreeMap<crate::core::AgentRunId, super::blockers::BlockerWatcher>,
) {
    std::mem::take(&mut runtime.blocker_watchers)
        .into_iter()
        .partition(|(_, watcher)| watcher.host_id == host_id)
}

fn move_host_entries(from: &mut ClientRuntime, to: &mut ClientRuntime, host_id: HostId) {
    if let Some(helper) = from.helper_paths.remove(&host_id) {
        to.helper_paths.insert(host_id, helper);
    }
    if let Some(pending) = from.remote_pending_worktree_removals.remove(&host_id) {
        to.remote_pending_worktree_removals.insert(host_id, pending);
    }
    for (run_id, watcher) in std::mem::take(&mut from.blocker_watchers) {
        to.blocker_watchers.insert(run_id, watcher);
    }
}

fn ssh_destination(host: &HostRecord) -> Option<String> {
    match &host.transport {
        crate::core::HostTransport::Ssh { destination } => Some(destination.clone()),
        crate::core::HostTransport::Local => None,
    }
}

#[cfg(test)]
#[path = "restore_tests.rs"]
mod tests;
