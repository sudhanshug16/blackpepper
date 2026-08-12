use super::ConnectionRestoreRuntime;
use crate::client::runtime::ClientRuntime;
use crate::core::HostRegistry;
use crate::transport::CommandCancellation;
use std::collections::BTreeMap;

impl ConnectionRestoreRuntime {
    pub(super) fn initialize(&mut self, cancellation: &CommandCancellation) -> Result<(), String> {
        if self.runtime.is_some() {
            return Ok(());
        }
        let pending = self
            .pending
            .as_ref()
            .ok_or_else(|| "The SSH restore payload is unavailable.".to_owned())?;
        let registry =
            HostRegistry::open_existing_interruptible(pending.paths.registry_path(), || {
                cancellation.is_cancelled()
            })
            .map_err(|error| format!("Could not open the restore registry: {error}"))?;
        let pending = self
            .pending
            .take()
            .expect("checked restore payload vanished");
        let host_id = self.host_id;
        self.runtime = Some(ClientRuntime {
            paths: pending.paths,
            registry,
            local_host_id: pending.local_host_id,
            hosts: BTreeMap::from([(host_id, pending.slot)]),
            helper_paths: pending
                .helper
                .into_iter()
                .map(|path| (host_id, path))
                .collect(),
            remote_pending_worktree_removals: pending
                .removals
                .into_iter()
                .map(|removals| (host_id, removals))
                .collect(),
            local_port_proxies: BTreeMap::new(),
            blocker_watchers: pending.watchers,
            connection_restores: BTreeMap::new(),
            host_operations: BTreeMap::new(),
            host_operation_generations: BTreeMap::new(),
            deferred_host_actions: BTreeMap::new(),
            startup_warnings: Vec::new(),
            _singleton: None,
        });
        Ok(())
    }

    pub(super) fn runtime_mut(&mut self) -> &mut ClientRuntime {
        self.runtime
            .as_mut()
            .expect("restore runtime must be initialized on its worker")
    }

    pub(super) fn abort(&mut self) {
        if let Some(runtime) = self.runtime.as_mut() {
            super::super::connection::abort(runtime, self.host_id);
        } else {
            self.pending.take();
        }
    }
}
