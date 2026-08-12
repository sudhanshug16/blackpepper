use super::super::{blockers::BlockerWatcher, ClientRuntime, HostSlot};
use crate::core::{HostId, HostRegistry, WorkspaceId};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct HostOperationRuntime {
    pending: Option<PendingHostOperationRuntime>,
    runtime: Option<ClientRuntime>,
}

struct PendingHostOperationRuntime {
    paths: crate::core::CorePaths,
    local_host_id: HostId,
    host_id: HostId,
    slot: HostSlot,
    helper: Option<String>,
    removals: Option<BTreeSet<WorkspaceId>>,
    local_port_proxies: super::super::local_proxy::LocalPortProxies,
    watchers: BTreeMap<crate::core::AgentRunId, BlockerWatcher>,
}

impl HostOperationRuntime {
    pub(super) fn initialize(&mut self) -> Result<&mut ClientRuntime, String> {
        if self.runtime.is_none() {
            let pending = self
                .pending
                .as_ref()
                .ok_or_else(|| "The operation host payload is unavailable.".to_owned())?;
            let registry = HostRegistry::open_existing_interruptible(
                pending.paths.registry_path(),
                crate::transport::CommandCancellation::scope_is_cancelled,
            )
            .map_err(|error| format!("Could not open the operation registry: {error}"))?;
            let pending = self.pending.take().expect("checked payload disappeared");
            let host_id = pending.host_id;
            self.runtime = Some(ClientRuntime {
                paths: pending.paths,
                registry,
                local_host_id: pending.local_host_id,
                hosts: BTreeMap::from([(host_id, pending.slot)]),
                helper_paths: pending.helper.into_iter().map(|v| (host_id, v)).collect(),
                remote_pending_worktree_removals: pending
                    .removals
                    .into_iter()
                    .map(|v| (host_id, v))
                    .collect(),
                local_port_proxies: pending.local_port_proxies,
                blocker_watchers: pending.watchers,
                connection_restores: BTreeMap::new(),
                host_operations: BTreeMap::new(),
                host_operation_generations: BTreeMap::new(),
                deferred_host_actions: BTreeMap::new(),
                startup_warnings: Vec::new(),
                _singleton: None,
            });
        }
        Ok(self.runtime.as_mut().expect("runtime initialized"))
    }

    pub(super) fn snapshot(&self) -> Result<crate::core::RegistrySnapshot, String> {
        self.runtime
            .as_ref()
            .ok_or_else(|| "The operation registry did not initialize.".to_owned())?
            .snapshot()
    }
}

impl ClientRuntime {
    pub(super) fn split_host_operation(
        &mut self,
        host_id: HostId,
    ) -> Result<HostOperationRuntime, String> {
        let local = host_id == self.local_host_id;
        let slot = if local {
            HostSlot::Local(crate::transport::LocalTransport)
        } else {
            self.hosts
                .remove(&host_id)
                .ok_or_else(|| format!("Host {host_id} is not connected."))?
        };
        let helper = self.helper_paths.remove(&host_id);
        let removals = self.remote_pending_worktree_removals.remove(&host_id);
        let (watchers, retained): (BTreeMap<_, BlockerWatcher>, BTreeMap<_, BlockerWatcher>) =
            std::mem::take(&mut self.blocker_watchers)
                .into_iter()
                .partition(|(_, watcher)| watcher.host_id == host_id);
        self.blocker_watchers = retained;
        let local_port_proxies = if local {
            std::mem::take(&mut self.local_port_proxies)
        } else {
            BTreeMap::new()
        };
        Ok(HostOperationRuntime {
            pending: Some(PendingHostOperationRuntime {
                paths: self.paths.clone(),
                local_host_id: self.local_host_id,
                host_id,
                slot,
                helper,
                removals,
                local_port_proxies,
                watchers,
            }),
            runtime: None,
        })
    }

    pub(super) fn merge_host_operation_runtime(
        &mut self,
        host_id: HostId,
        owned: &mut HostOperationRuntime,
    ) -> Result<(), String> {
        if owned.runtime.is_none() {
            return self.merge_pending_operation(host_id, owned);
        }
        let runtime = owned.runtime.as_mut().expect("checked runtime disappeared");
        if host_id != self.local_host_id {
            if self.hosts.contains_key(&host_id) {
                return Err("a newer connection already owns this host".into());
            }
            let slot = runtime
                .hosts
                .remove(&host_id)
                .ok_or_else(|| "the operation returned without its host transport".to_owned())?;
            self.hosts.insert(host_id, slot);
        }
        if let Some(helper) = runtime.helper_paths.remove(&host_id) {
            self.helper_paths.insert(host_id, helper);
        }
        if let Some(removals) = runtime.remote_pending_worktree_removals.remove(&host_id) {
            self.remote_pending_worktree_removals
                .insert(host_id, removals);
        }
        if host_id == self.local_host_id {
            self.local_port_proxies
                .append(&mut runtime.local_port_proxies);
        }
        self.blocker_watchers
            .extend(std::mem::take(&mut runtime.blocker_watchers));
        Ok(())
    }

    fn merge_pending_operation(
        &mut self,
        host_id: HostId,
        owned: &mut HostOperationRuntime,
    ) -> Result<(), String> {
        let pending = owned
            .pending
            .take()
            .ok_or_else(|| "the operation returned without its host payload".to_owned())?;
        if host_id != self.local_host_id {
            if self.hosts.contains_key(&host_id) {
                return Err("a newer connection already owns this host".into());
            }
            self.hosts.insert(host_id, pending.slot);
        }
        if let Some(helper) = pending.helper {
            self.helper_paths.insert(host_id, helper);
        }
        if let Some(removals) = pending.removals {
            self.remote_pending_worktree_removals
                .insert(host_id, removals);
        }
        if host_id == self.local_host_id {
            self.local_port_proxies.extend(pending.local_port_proxies);
        }
        self.blocker_watchers.extend(pending.watchers);
        Ok(())
    }
}
