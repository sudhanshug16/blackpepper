mod validation;

use super::super::{ClientRuntime, HostSlot};
use super::operations::{handshake_host_id, helper_exchange};
use super::remote_helper::find_helper;
use crate::core::{HostId, HostRecord, ResponsePayload, ResponseResult};
use std::collections::BTreeSet;
use validation::validate_remote_snapshot;

#[cfg(test)]
pub(super) use validation::{validate_remote_host_identity, validate_reserved_host_identity};
#[cfg(not(test))]
use validation::{validate_remote_host_identity, validate_reserved_host_identity};

pub(in crate::client::runtime) fn synchronize_registry_with_reserved(
    runtime: &mut ClientRuntime,
    old_id: HostId,
    reserved_host_ids: &BTreeSet<HostId>,
) -> Result<HostId, String> {
    let (alias, helper, responses) = {
        let Some(HostSlot::Ssh(host)) = runtime.hosts.get_mut(&old_id) else {
            return Err("SSH host disappeared during connection.".to_string());
        };
        let helper = find_helper(&mut host.transport)?;
        let responses = helper_exchange(&mut host.transport, &helper)?;
        (host.alias.clone(), helper, responses)
    };
    let remote_id = handshake_host_id(&responses)?;
    validate_reserved_host_identity(old_id, remote_id, reserved_host_ids)?;
    validate_remote_host_identity(runtime, old_id, remote_id)?;
    let remote_snapshot = responses
        .iter()
        .find_map(|response| match &response.result {
            ResponseResult::Ok {
                payload: ResponsePayload::Snapshot { snapshot },
            } => Some(snapshot.clone()),
            _ => None,
        })
        .ok_or_else(|| "bp-host did not return a registry snapshot.".to_string())?;
    // Validate every imported stable ID before changing the temporary host
    // record. A malformed or cloned remote registry must leave the local host
    // list and all cached workspace/session ownership untouched.
    validate_remote_snapshot(runtime, remote_id, &remote_snapshot)
        .map_err(|error| format!("validate remote registry snapshot: {error}"))?;

    let old_record = runtime
        .host_record(old_id)
        .map_err(|error| format!("read the cached SSH host: {error}"))?;
    let mut remote_record = HostRecord::new(alias, old_record.transport);
    remote_record.id = remote_id;
    runtime
        .registry
        .upsert_host(&remote_record)
        .map_err(|error| format!("cache the stable remote host identity: {error}"))?;
    // Once the helper has proved its stable installation identity, the
    // config-derived record is only a temporary connection key. Remove it
    // before importing metadata so a failed first sync cannot leave two hosts
    // with the same SSH destination and make the next retry ambiguous.
    if old_id != remote_id {
        runtime
            .registry
            .remove_host(old_id)
            .map_err(|error| format!("remove the temporary SSH host identity: {error}"))?;
    }
    reconcile_remote_snapshot(runtime, remote_id, &remote_snapshot)
        .map_err(|error| format!("import the remote registry snapshot: {error}"))?;

    if old_id != remote_id {
        let slot = runtime
            .hosts
            .remove(&old_id)
            .ok_or_else(|| "SSH host disappeared during identity update.".to_string())?;
        runtime.hosts.insert(remote_id, slot);
    }
    runtime.helper_paths.remove(&old_id);
    runtime.helper_paths.insert(remote_id, helper);
    if let Some(HostSlot::Ssh(host)) = runtime.hosts.get_mut(&remote_id) {
        host.registry_synchronized = true;
        host.registry_synchronizing = false;
    }
    Ok(remote_id)
}

pub(in crate::client::runtime) fn reconcile_remote_snapshot(
    runtime: &mut ClientRuntime,
    remote_id: HostId,
    remote_snapshot: &crate::core::RegistrySnapshot,
) -> Result<(), String> {
    validate_remote_snapshot(runtime, remote_id, remote_snapshot)?;

    let remote_workspace_ids = remote_snapshot
        .workspaces
        .iter()
        .map(|workspace| workspace.id)
        .collect::<BTreeSet<_>>();
    let pending_removals = remote_snapshot
        .pending_worktree_removals
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    for workspace in &remote_snapshot.workspaces {
        runtime
            .registry
            .upsert_workspace(workspace)
            .map_err(|error| error.to_string())?;
    }
    for session in &remote_snapshot.sessions {
        runtime
            .registry
            .upsert_session(session)
            .map_err(|error| error.to_string())?;
    }
    let remote_session_ids = remote_snapshot
        .sessions
        .iter()
        .map(|session| session.id)
        .collect::<BTreeSet<_>>();
    let cached = runtime
        .registry
        .snapshot()
        .map_err(|error| error.to_string())?;
    for session in cached.sessions.iter().filter(|session| {
        remote_workspace_ids.contains(&session.workspace_id)
            && !remote_session_ids.contains(&session.id)
    }) {
        runtime
            .registry
            .remove_session(session.id)
            .map_err(|error| error.to_string())?;
    }
    for workspace in cached.workspaces.iter().filter(|workspace| {
        workspace.host_id == remote_id && !remote_workspace_ids.contains(&workspace.id)
    }) {
        runtime
            .registry
            .remove_workspace(workspace.id)
            .map_err(|error| error.to_string())?;
    }

    if pending_removals.is_empty() {
        runtime.remote_pending_worktree_removals.remove(&remote_id);
    } else {
        runtime
            .remote_pending_worktree_removals
            .insert(remote_id, pending_removals);
    }

    Ok(())
}
