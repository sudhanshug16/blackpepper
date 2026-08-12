use super::{connection, ClientRuntime};
use crate::core::{HostId, RegistrySnapshot, WorkspaceId, WorkspaceRecord};
use crate::ports::{ForwardState, ForwardStatus, PortSnapshot, RemotePortTarget};
use crate::transport::{LocalForward, TransportError};
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ForwardReconcileReport {
    pub removed: usize,
    pub failures: Vec<String>,
}

impl ClientRuntime {
    pub(crate) fn discover_ports(&mut self, host_id: HostId) -> Result<PortSnapshot, String> {
        match connection::registry_operation(
            self,
            host_id,
            crate::core::RequestOperation::DiscoverPorts,
        )? {
            crate::core::ResponsePayload::HostService { payload } => match *payload {
                crate::core::HostServicePayload::Ports { snapshot } => Ok(snapshot),
                _ => Err("bp-host returned an unexpected port-discovery response.".to_string()),
            },
            _ => Err("bp-host returned an unexpected port-discovery response.".to_string()),
        }
    }

    pub(crate) fn forward_workspace_port(
        &mut self,
        workspace_id: WorkspaceId,
        target: RemotePortTarget,
    ) -> Result<ForwardState, String> {
        // Reject a known recovery marker before starting another helper. The
        // host-side lease then closes the stale-client gap: it validates the
        // marker again on the workspace host and refreshes the client cache
        // while holding the same gate as Worktrunk removal.
        forwardable_workspace(&self.snapshot()?, workspace_id)?;
        let (lease, workspace) = self.acquire_workspace_session_lease(workspace_id)?;
        let workspace = forwardable_workspace(&self.snapshot()?, workspace.id)?;
        // Do not create a client-owned tunnel until the helper has confirmed
        // that its host lock was released. Otherwise a release error would
        // return no ForwardState while leaving an untracked tunnel alive.
        lease.release()?;
        if workspace.host_id == self.local_host_id {
            let remote =
                super::local_proxy::target_socket(&target.remote_host, target.remote_port)?;
            if remote.ip().is_loopback() {
                return Ok(ForwardState {
                    id: uuid::Uuid::new_v4(),
                    host_id: workspace.host_id,
                    workspace_id,
                    remote_host: target.remote_host,
                    remote_port: target.remote_port,
                    requested_local_address: remote,
                    local_address: remote,
                    status: ForwardStatus::Direct,
                });
            }
            let state = ForwardState::new(workspace.host_id, workspace_id, target)
                .map_err(|error| error.to_string())?;
            let proxy = super::local_proxy::LocalPortProxy::start(state.local_address, remote)
                .map_err(|error| format!("Could not start the local loopback proxy: {error}"))?;
            self.local_port_proxies.insert(state.local_address, proxy);
            return Ok(state);
        }
        let state = ForwardState::new(workspace.host_id, workspace_id, target)
            .map_err(|error| error.to_string())?;
        self.transport_mut(workspace.host_id)?
            .forward_local_port(local_forward(&state))
            .map_err(|error| error.to_string())?;
        Ok(state)
    }

    pub(crate) fn cancel_workspace_forward(
        &mut self,
        workspace_id: WorkspaceId,
        forward: &ForwardState,
    ) -> Result<(), String> {
        if workspace_id != forward.workspace_id {
            return Err("The selected forward belongs to another workspace.".to_string());
        }
        self.cancel_forward_by_host(forward)
    }

    /// Cancel every client-owned tunnel for a workspace before dispatching a
    /// destructive Worktrunk removal. Any live-master cancellation failure
    /// keeps that forward tracked and blocks the removal.
    pub(crate) fn cancel_workspace_forwards(
        &mut self,
        forwards: &mut Vec<ForwardState>,
        workspace_id: WorkspaceId,
    ) -> Result<usize, String> {
        let mut cancelled = 0;
        let mut failures = Vec::new();
        let mut retained = Vec::with_capacity(forwards.len());
        for mut forward in std::mem::take(forwards) {
            if forward.workspace_id != workspace_id {
                retained.push(forward);
                continue;
            }
            match self.cancel_forward_by_host(&forward) {
                Ok(()) => cancelled += 1,
                Err(error) => {
                    let message = format!(
                        "Could not stop forward {} before worktree removal: {error}",
                        forward.remote_endpoint()
                    );
                    forward.status = ForwardStatus::Failed(message.clone());
                    failures.push(message);
                    retained.push(forward);
                }
            }
        }
        *forwards = retained;
        if failures.is_empty() {
            Ok(cancelled)
        } else {
            Err(format!(
                "Worktree removal was blocked because {} client forward(s) could not be stopped: {}",
                failures.len(),
                failures.join(" | ")
            ))
        }
    }

    /// Stop and forget tunnels whose owning workspace disappeared from the
    /// host-authoritative registry. A failed cancellation remains visible and
    /// is retried on the next reconciliation rather than becoming an
    /// untracked live tunnel.
    pub(crate) fn reconcile_forwards(
        &mut self,
        forwards: &mut Vec<ForwardState>,
        snapshot: &RegistrySnapshot,
    ) -> ForwardReconcileReport {
        let mut report = ForwardReconcileReport::default();
        let mut retained = Vec::with_capacity(forwards.len());
        for mut forward in std::mem::take(forwards) {
            if forward_workspace_is_registered(snapshot, &forward) {
                retained.push(forward);
                continue;
            }

            match self.cancel_forward_by_host(&forward) {
                Ok(()) => report.removed += 1,
                Err(error) => {
                    let message = format!(
                        "Could not stop orphaned forward {} for removed workspace {}: {error}",
                        forward.remote_endpoint(),
                        forward.workspace_id
                    );
                    forward.status = ForwardStatus::Failed(message.clone());
                    report.failures.push(message);
                    retained.push(forward);
                }
            }
        }
        *forwards = retained;
        report
    }

    fn cancel_forward_by_host(&mut self, forward: &ForwardState) -> Result<(), String> {
        if forward.host_id == self.local_host_id {
            self.local_port_proxies.remove(&forward.local_address);
            return Ok(());
        }
        let Some(host) = self.hosts.get_mut(&forward.host_id) else {
            // A disconnected/failed transport has already dropped its
            // foreground ControlMaster, which also removes every owned
            // forward. There is no live tunnel left to cancel.
            return Ok(());
        };
        match host
            .transport_mut()
            .cancel_local_forward(&local_forward(forward))
        {
            Ok(()) | Err(TransportError::ForwardNotOwned(_)) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }

    /// Recreate an SSH forward on the exact client port selected initially.
    /// Reconnects never silently move a URL to a different port.
    pub(crate) fn reconnect_forward(&mut self, forward: &mut ForwardState) {
        let registered = self
            .snapshot()
            .map(|snapshot| forward_workspace_is_registered(&snapshot, forward));
        match registered {
            Ok(true) => {}
            Ok(false) => {
                forward.status = ForwardStatus::Failed(
                    "Owning workspace was removed; forward was not restored.".to_string(),
                );
                return;
            }
            Err(error) => {
                forward.status = ForwardStatus::Failed(format!(
                    "Could not verify the owning workspace; forward was not restored: {error}"
                ));
                return;
            }
        }
        if forward.host_id == self.local_host_id {
            let target = match super::local_proxy::target_socket(
                &forward.remote_host,
                forward.remote_port,
            ) {
                Ok(target) => target,
                Err(error) => {
                    forward.status = ForwardStatus::Failed(error);
                    return;
                }
            };
            if target.ip().is_loopback() {
                forward.status = ForwardStatus::Direct;
                return;
            }
            if self.local_port_proxies.contains_key(&forward.local_address) {
                forward.status = ForwardStatus::Active;
                return;
            }
            if !crate::ports::port_is_available(forward.local_address.port()) {
                forward.status = ForwardStatus::PortConflict;
                return;
            }
            forward.status =
                match super::local_proxy::LocalPortProxy::start(forward.local_address, target) {
                    Ok(proxy) => {
                        self.local_port_proxies.insert(forward.local_address, proxy);
                        ForwardStatus::Active
                    }
                    Err(error) => ForwardStatus::Failed(error.to_string()),
                };
            return;
        }
        if !crate::ports::port_is_available(forward.local_address.port()) {
            forward.status = ForwardStatus::PortConflict;
            return;
        }
        let requested = local_forward(forward);
        forward.status = match self.transport_mut(forward.host_id) {
            Ok(transport) => match transport.forward_local_port(requested) {
                Ok(_) => ForwardStatus::Active,
                Err(error) => ForwardStatus::Failed(error.to_string()),
            },
            Err(error) => ForwardStatus::Failed(error),
        };
    }
}

fn forwardable_workspace(
    snapshot: &RegistrySnapshot,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceRecord, String> {
    if snapshot.pending_worktree_removals.contains(&workspace_id) {
        return Err(
            "This workspace has a Worktrunk removal with an unknown result; run :worktree list before forwarding its ports."
                .to_owned(),
        );
    }
    snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.id == workspace_id)
        .cloned()
        .ok_or_else(|| "The selected workspace no longer exists.".to_owned())
}

pub(super) fn forward_workspace_is_registered(
    snapshot: &RegistrySnapshot,
    forward: &ForwardState,
) -> bool {
    !snapshot
        .pending_worktree_removals
        .contains(&forward.workspace_id)
        && snapshot.workspaces.iter().any(|workspace| {
            workspace.id == forward.workspace_id && workspace.host_id == forward.host_id
        })
}

pub(super) fn local_forward(forward: &ForwardState) -> LocalForward {
    LocalForward {
        bind_address: IpAddr::V4(Ipv4Addr::LOCALHOST),
        local_port: forward.local_address.port(),
        remote_host: forward.remote_host.clone(),
        remote_port: forward.remote_port,
    }
}

pub(crate) fn listener_matches_workspace(listener_path: Option<&Path>, root: &str) -> bool {
    listener_path.is_some_and(|path| path.starts_with(Path::new(root)))
}

#[cfg(test)]
mod tests;
