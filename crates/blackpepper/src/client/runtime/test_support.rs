use super::{ClientRuntime, HostSlot, SshHost};
use crate::core::{CorePaths, HostId, HostRecord, HostRegistry, HostTransport as StoredTransport};
use crate::transport::{LocalTransport, SshConfig, SshTransport};
use std::collections::BTreeMap;
use std::path::Path;

impl ClientRuntime {
    pub(crate) fn test_fixture(root: &Path) -> Self {
        let paths = CorePaths::from_roots(root.join("state"), root.join("run"));
        paths.prepare().unwrap();
        let mut registry = HostRegistry::open(paths.registry_path()).unwrap();
        let local_host_id = registry.ensure_local_host("test-local").unwrap();
        Self {
            paths,
            registry,
            local_host_id,
            hosts: BTreeMap::from([(local_host_id, HostSlot::Local(LocalTransport))]),
            helper_paths: BTreeMap::new(),
            remote_pending_worktree_removals: BTreeMap::new(),
            local_port_proxies: BTreeMap::new(),
            blocker_watchers: BTreeMap::new(),
            connection_restores: BTreeMap::new(),
            host_operations: BTreeMap::new(),
            host_operation_generations: BTreeMap::new(),
            deferred_host_actions: BTreeMap::new(),
            startup_warnings: Vec::new(),
            _singleton: None,
        }
    }

    pub(crate) fn test_add_ssh_slot(&mut self, name: &str, destination: &str) -> HostId {
        let record = HostRecord::new(
            name,
            StoredTransport::Ssh {
                destination: destination.to_owned(),
            },
        );
        self.registry.upsert_host(&record).unwrap();
        self.hosts.insert(
            record.id,
            HostSlot::Ssh(Box::new(SshHost {
                alias: name.to_owned(),
                transport: SshTransport::new(SshConfig::new(destination)).unwrap(),
                registry_synchronized: false,
                registry_synchronizing: true,
            })),
        );
        record.id
    }

    pub(crate) fn test_connection_can_start(&self, host_id: HostId) -> bool {
        let Ok(record) = self.host_record(host_id) else {
            return false;
        };
        !self.hosts.contains_key(&host_id) && !self.connection_restore_matches(&record)
    }
}
