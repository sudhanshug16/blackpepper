use super::super::super::{ClientRuntime, HostSlot};
use crate::core::HostId;
use std::collections::BTreeSet;

pub(in crate::client::runtime::connection) fn validate_reserved_host_identity(
    connection_id: HostId,
    remote_id: HostId,
    reserved_host_ids: &BTreeSet<HostId>,
) -> Result<(), String> {
    if remote_id == connection_id || !reserved_host_ids.contains(&remote_id) {
        return Ok(());
    }
    Err(format!(
        "The remote host reported installation ID {remote_id}, but another live connection already owns that identity. The new connection was discarded without importing its registry."
    ))
}

/// Refuse identity collisions before mutating the client registry or moving a
/// live transport. A copied XDG state directory can legitimately reproduce a
/// UUID, so this boundary must not rely on collision probability.
pub(in crate::client::runtime::connection) fn validate_remote_host_identity(
    runtime: &ClientRuntime,
    connection_id: HostId,
    remote_id: HostId,
) -> Result<(), String> {
    if remote_id == runtime.local_host_id {
        return Err(
            "The remote host reported this client's local installation ID. Its Blackpepper state was likely cloned; reset the remote Blackpepper host identity before reconnecting."
                .to_owned(),
        );
    }
    if connection_id != remote_id && matches!(runtime.hosts.get(&remote_id), Some(HostSlot::Ssh(_)))
    {
        return Err(format!(
            "The remote host reported installation ID {remote_id}, but a different live SSH connection already owns that identity. Disconnect the existing host or repair the cloned remote state before retrying."
        ));
    }
    Ok(())
}

pub(super) fn validate_remote_snapshot(
    runtime: &ClientRuntime,
    remote_id: HostId,
    remote_snapshot: &crate::core::RegistrySnapshot,
) -> Result<(), String> {
    if remote_snapshot
        .workspaces
        .iter()
        .any(|workspace| workspace.host_id != remote_id)
    {
        return Err("bp-host returned a workspace owned by another host.".to_string());
    }
    let remote_workspace_ids = remote_snapshot
        .workspaces
        .iter()
        .map(|workspace| workspace.id)
        .collect::<BTreeSet<_>>();
    if remote_workspace_ids.len() != remote_snapshot.workspaces.len() {
        return Err("bp-host returned duplicate workspace IDs.".to_owned());
    }
    let remote_workspace_paths = remote_snapshot
        .workspaces
        .iter()
        .map(|workspace| workspace.root_path.as_str())
        .collect::<BTreeSet<_>>();
    if remote_workspace_paths.len() != remote_snapshot.workspaces.len() {
        return Err("bp-host returned duplicate workspace paths.".to_owned());
    }
    if remote_snapshot
        .sessions
        .iter()
        .any(|session| !remote_workspace_ids.contains(&session.workspace_id))
    {
        return Err("bp-host returned a session outside its workspace snapshot.".to_string());
    }
    let remote_session_ids = remote_snapshot
        .sessions
        .iter()
        .map(|session| session.id)
        .collect::<BTreeSet<_>>();
    if remote_session_ids.len() != remote_snapshot.sessions.len() {
        return Err("bp-host returned duplicate session IDs.".to_owned());
    }
    let pending_removals = remote_snapshot
        .pending_worktree_removals
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if !pending_removals.is_subset(&remote_workspace_ids) {
        return Err(
            "bp-host returned a pending Worktrunk removal outside its workspace snapshot."
                .to_string(),
        );
    }
    if pending_removals.len() != remote_snapshot.pending_worktree_removals.len() {
        return Err("bp-host returned duplicate pending Worktrunk removals.".to_owned());
    }

    let cached = runtime
        .registry
        .snapshot()
        .map_err(|error| error.to_string())?;
    for workspace in &remote_snapshot.workspaces {
        if let Some(existing) = cached
            .workspaces
            .iter()
            .find(|existing| existing.id == workspace.id)
        {
            if existing.host_id != remote_id {
                return Err(format!(
                    "Remote workspace ID {} already belongs to host {}; registry import was refused.",
                    workspace.id, existing.host_id
                ));
            }
        }
        if let Some(existing) = cached.workspaces.iter().find(|existing| {
            existing.host_id == remote_id
                && existing.root_path == workspace.root_path
                && existing.id != workspace.id
        }) {
            return Err(format!(
                "Remote workspace path {} changed identity from {} to {}; registry import was refused.",
                workspace.root_path, existing.id, workspace.id
            ));
        }
        if cached.pending_worktree_removals.contains(&workspace.id) {
            return Err(format!(
                "Remote workspace ID {} collides with a local pending Worktrunk removal; registry import was refused.",
                workspace.id
            ));
        }
        if runtime
            .remote_pending_worktree_removals
            .iter()
            .any(|(host_id, removals)| *host_id != remote_id && removals.contains(&workspace.id))
        {
            return Err(format!(
                "Remote workspace ID {} collides with another host's pending Worktrunk removal; registry import was refused.",
                workspace.id
            ));
        }
    }
    for session in &remote_snapshot.sessions {
        if let Some(existing) = cached
            .sessions
            .iter()
            .find(|existing| existing.id == session.id)
        {
            if existing.workspace_id != session.workspace_id {
                return Err(format!(
                    "Remote session ID {} already belongs to workspace {}; registry import was refused.",
                    session.id, existing.workspace_id
                ));
            }
        }
    }

    Ok(())
}
