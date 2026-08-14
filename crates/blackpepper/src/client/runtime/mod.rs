//! Mutable host/session state kept outside the render model.

mod agent_assets;
mod agent_lifecycle;
mod agents;
mod blockers;
mod connection;
mod forward_cleanup;
mod helper;
mod local_proxy;
mod operation;
mod panes;
mod periodic;
pub(super) mod ports;
mod restore;
mod services;
mod session_lease;
mod startup;
mod terminal_identity;
#[cfg(test)]
mod test_support;
mod workspace;
mod worktrunk;

use crate::client::ClientEvent;
use crate::client_config::ClientConfig;
use crate::core::{
    CorePaths, HostId, HostRecord, HostRegistry, HostTransport as StoredTransport,
    RegistrySnapshot, SingletonLock,
};
use crate::transport::{HostTransport, LocalTransport, SshTransport};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

pub(crate) use agents::SpawnedAgent;
pub(crate) use connection::ConnectionUpdate;
pub(crate) use forward_cleanup::{ForwardCleanupBatch, ForwardCleanupOutcome};
pub(crate) use operation::{
    DeferredHostAction, DeferredHostResult, DurableActionQueue, HostOperationContext,
    HostOperationValue, WorktreeMutationResult,
};
pub(crate) use periodic::PeriodicRefreshJob;
pub(crate) use restore::{ConnectionRestoreReport, ConnectionRestoreRuntime};
pub(crate) use worktrunk::WorktreeChange;

pub(crate) struct ClientRuntime {
    pub(super) paths: CorePaths,
    pub(super) registry: HostRegistry,
    pub(super) local_host_id: HostId,
    pub(super) hosts: BTreeMap<HostId, HostSlot>,
    pub(super) helper_paths: BTreeMap<HostId, String>,
    pub(super) remote_pending_worktree_removals:
        BTreeMap<HostId, std::collections::BTreeSet<crate::core::WorkspaceId>>,
    local_port_proxies: local_proxy::LocalPortProxies,
    blocker_watchers: BTreeMap<crate::core::AgentRunId, blockers::BlockerWatcher>,
    connection_restores: BTreeMap<HostId, restore::ActiveConnectionRestore>,
    host_operations: BTreeMap<HostId, operation::ActiveHostOperation>,
    host_operation_generations: BTreeMap<HostId, u64>,
    deferred_host_actions: BTreeMap<HostId, Vec<operation::DeferredHostAction>>,
    startup_warnings: Vec<String>,
    // Restore workers never acquire or release the interactive singleton.
    _singleton: Option<SingletonLock>,
}

pub(crate) struct HostDisconnectReport {
    pub restoring_host_ids: Vec<HostId>,
    pub deferred_results: Vec<operation::DeferredHostResult>,
    pub warning: Option<String>,
}

pub(super) enum HostSlot {
    Local(LocalTransport),
    Ssh(Box<SshHost>),
}

pub(super) struct SshHost {
    pub alias: String,
    pub transport: SshTransport,
    pub registry_synchronized: bool,
    pub registry_synchronizing: bool,
}

impl HostSlot {
    pub(super) fn transport_mut(&mut self) -> &mut dyn HostTransport {
        match self {
            Self::Local(transport) => transport,
            Self::Ssh(host) => &mut host.transport,
        }
    }
}

impl ClientRuntime {
    pub(crate) fn initialize(
        cwd: &Path,
        config: &ClientConfig,
    ) -> Result<(Self, RegistrySnapshot), String> {
        let paths = CorePaths::discover().map_err(|error| error.to_string())?;
        paths.prepare().map_err(|error| error.to_string())?;
        let singleton = SingletonLock::acquire(paths.singleton_lock_path())
            .map_err(|error| error.to_string())?;
        let mut registry =
            HostRegistry::open(paths.registry_path()).map_err(|error| error.to_string())?;
        let local_host_id = registry
            .ensure_local_host(&local_display_name())
            .map_err(|error| error.to_string())?;

        for (name, host_config) in &config.hosts {
            let destination = host_config.destination(name);
            let existing = registry
                .snapshot()
                .map_err(|error| error.to_string())?
                .hosts
                .into_iter()
                .find(|host| {
                    matches!(
                        &host.transport,
                        StoredTransport::Ssh { destination: value } if value == destination
                    )
                });
            if existing.is_none() {
                registry
                    .upsert_host(&HostRecord::new(
                        name,
                        StoredTransport::Ssh {
                            destination: destination.to_string(),
                        },
                    ))
                    .map_err(|error| error.to_string())?;
            }
        }

        let mut runtime = Self {
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
            _singleton: Some(singleton),
        };
        if cwd.is_dir() {
            runtime.register_workspace(local_host_id, cwd)?;
        }
        if let Err(error) = runtime.restore_host_workspaces(local_host_id) {
            runtime.startup_warnings.push(error);
        }
        let snapshot = runtime.snapshot()?;
        Ok((runtime, snapshot))
    }

    pub(crate) fn take_startup_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.startup_warnings)
    }

    pub(crate) fn snapshot(&self) -> Result<RegistrySnapshot, String> {
        let mut snapshot = self
            .registry
            .snapshot()
            .map_err(|error| error.to_string())?;
        snapshot.pending_worktree_removals.extend(
            self.remote_pending_worktree_removals
                .values()
                .flatten()
                .copied(),
        );
        snapshot.pending_worktree_removals.sort_unstable();
        snapshot.pending_worktree_removals.dedup();
        Ok(snapshot)
    }

    pub(crate) fn local_host_id(&self) -> HostId {
        self.local_host_id
    }

    pub(crate) fn add_ssh_host(&mut self, name: &str, destination: &str) -> Result<HostId, String> {
        let hosts = self.snapshot()?.hosts;
        if let Some(host) = hosts.iter().find(|host| {
            matches!(
                &host.transport,
                StoredTransport::Ssh { destination: value } if value == destination
            )
        }) {
            return Ok(host.id);
        }
        if hosts.iter().any(|host| host.display_name == name) {
            return Err(format!(
                "A host named '{name}' is already registered; choose a different name."
            ));
        }
        let host = HostRecord::new(
            name,
            StoredTransport::Ssh {
                destination: destination.to_string(),
            },
        );
        self.registry
            .upsert_host(&host)
            .map_err(|error| error.to_string())?;
        Ok(host.id)
    }

    pub(crate) fn find_host(&self, selector: &str) -> Result<HostRecord, String> {
        let snapshot = self.snapshot()?;
        let mut matching = snapshot.hosts.into_iter().filter(|host| {
            host.display_name == selector
                || host.id.to_string() == selector
                || host.id.to_string().starts_with(selector)
        });
        let host = matching
            .next()
            .ok_or_else(|| format!("No host matches '{selector}'."))?;
        if matching.next().is_some() {
            return Err(format!("Host selector '{selector}' is ambiguous."));
        }
        Ok(host)
    }

    pub(super) fn host_record(&self, host_id: HostId) -> Result<HostRecord, String> {
        self.registry
            .host(host_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Host {host_id} is no longer registered."))
    }

    pub(super) fn transport_mut(
        &mut self,
        host_id: HostId,
    ) -> Result<&mut dyn HostTransport, String> {
        if let Some(label) = self.host_operation_label(host_id) {
            return Err(format!(
                "Host {host_id} is busy with {label}; wait for it to finish or press Esc to cancel it."
            ));
        }
        self.hosts
            .get_mut(&host_id)
            .map(HostSlot::transport_mut)
            .ok_or_else(|| format!("Host {host_id} is not connected."))
    }

    pub(crate) fn start_connection(
        &mut self,
        host: HostRecord,
        sender: Sender<ClientEvent>,
    ) -> Result<(), String> {
        if let Some(label) = self.host_operation_label(host.id) {
            return Err(format!(
                "{} is busy with {label}; wait for it to finish or cancel it before reconnecting.",
                host.display_name
            ));
        }
        if self.connection_restore_matches(&host) {
            return Err(format!(
                "SSH recovery for {} is already running; disconnect it before starting a new connection.",
                host.display_name
            ));
        }
        connection::start(self, host, sender)
    }

    pub(crate) fn poll_connections(&mut self) -> Vec<ConnectionUpdate> {
        connection::poll(self)
    }

    pub(crate) fn send_authentication_input(
        &mut self,
        host_id: HostId,
        bytes: &[u8],
    ) -> Result<(), String> {
        connection::send_input(self, host_id, bytes)
    }

    pub(crate) fn disconnect_host_with_restores(
        &mut self,
        host_id: HostId,
    ) -> Result<HostDisconnectReport, String> {
        let restoring = self.cancel_connection_restores(host_id);
        let warning = self.disconnect_operation_warning(host_id);
        self.cancel_host_operation_for_disconnect(host_id);
        self.stop_blocker_watchers(host_id);
        connection::disconnect(self, host_id)?;
        let deferred_results = self.fail_queued_durable_actions(
            host_id,
            "The host disconnected before durable terminal state could be written.",
        );
        Ok(HostDisconnectReport {
            restoring_host_ids: restoring,
            deferred_results,
            warning,
        })
    }

    /// Drop an unusable connection; its owned PTY kills and reaps the master.
    pub(crate) fn abort_host_connection(&mut self, host_id: HostId) {
        self.stop_blocker_watchers(host_id);
        connection::abort(self, host_id);
    }
}

fn local_display_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "local".to_string())
}

pub(super) fn text_path(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("Path is not valid UTF-8: {}", path.display()))
}

pub(super) fn canonical_local(path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path)
        .map_err(|error| format!("Could not open workspace {}: {error}", path.display()))
}
