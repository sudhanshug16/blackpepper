use super::super::super::session_lease::SessionInitializationLease;
use super::super::super::worktrunk_approval::{authorize, ApprovalDecision};
use super::super::super::worktrunk_lock::repository_identity;
use super::super::{
    canonical_repository, execute, reject_declined_commands, require_success, WorktrunkExecutor,
};
use crate::core::{
    HostRegistry, HostServicePayload, SessionBackend, SessionState, WorkspaceId, WorkspaceRecord,
    WorktrunkMutationOutcome, WorktrunkRemovalIntent,
};
use crate::worktrunk::{Worktrunk, WorktrunkApprovalToken};
use std::path::{Path, PathBuf};

pub(super) fn remove(
    executor: &WorktrunkExecutor,
    registry: &HostRegistry,
    workspace_id: WorkspaceId,
    target_path: &str,
    approval: Option<&WorktrunkApprovalToken>,
) -> Result<HostServicePayload, String> {
    let intent = removal_intent(registry, workspace_id, target_path)?;
    let (surviving, target) = validate_git_removal_identity(&intent)?;
    let spec = Worktrunk::new(executor.binary()?).remove(&surviving, &target)?;

    // Previewing must remain possible while the workspace session is still
    // open; the approved call arrives only after the client terminates it.
    if approval.is_none() {
        return match authorize(
            executor.binary()?,
            &executor.lock_dir,
            &surviving,
            &spec,
            None,
        )? {
            ApprovalDecision::Required(payload) => Ok(*payload),
            ApprovalDecision::Authorized(_) => {
                Err("Worktrunk removal preview unexpectedly executed.".to_owned())
            }
        };
    }

    // Serialize against session creation after the client has terminated the
    // target. The durable removal marker is written before this lease can be
    // released by an abrupt helper exit, so a waiting session helper refuses
    // to recreate a removed or unknown workspace.
    let _session_lease =
        SessionInitializationLease::acquire_for_workspace(&executor.paths, workspace_id)?;
    let current = removal_intent(registry, workspace_id, target_path)?;
    if current != intent {
        return Err("Worktrunk workspace identity changed; removal was refused.".to_owned());
    }
    require_all_zellij_sessions_exited(registry, workspace_id)?;
    let (surviving, target) = validate_git_removal_identity(&intent)?;
    let lock = match authorize(
        executor.binary()?,
        &executor.lock_dir,
        &surviving,
        &spec,
        approval,
    )? {
        ApprovalDecision::Required(payload) => return Ok(*payload),
        ApprovalDecision::Authorized(lock) => lock,
    };
    let current = registry
        .plan_worktrunk_removal(
            intent.workspace_id,
            intent.surviving_workspace_id,
            &intent.target_path,
            intent.repository_key.clone(),
        )
        .map_err(|error| error.to_string())?;
    if current != intent {
        return Err("Worktrunk workspace identity changed; removal was refused.".to_owned());
    }
    validate_git_removal_identity(&intent)?;
    registry
        .journal_worktrunk_removal(&intent)
        .map_err(|error| error.to_string())?;
    let output = match execute(&lock, &spec) {
        Ok(output) => output,
        Err(error) => {
            return Err(format!(
                "{error} The Worktrunk removal result is unknown and was not retried; run :worktree list to reconcile it."
            ));
        }
    };
    if let Err(error) =
        reject_declined_commands(&output).and_then(|()| require_success(&spec, &output))
    {
        return Err(format!(
            "{error} The result was not retried; run :worktree list to reconcile it."
        ));
    }
    registry
        .finish_worktrunk_removal(&intent)
        .map_err(|error| {
            format!(
                "Worktrunk removed {}, but registry cleanup is pending: {error}. Run :worktree list to reconcile it.",
                target.display()
            )
        })?;
    Ok(HostServicePayload::WorktrunkMutation {
        outcome: WorktrunkMutationOutcome::Removed { path: target },
    })
}

fn require_all_zellij_sessions_exited(
    registry: &HostRegistry,
    workspace_id: WorkspaceId,
) -> Result<(), String> {
    let active = registry
        .sessions_for_workspace(workspace_id)
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|session| {
            matches!(session.backend, SessionBackend::Zellij)
                && session.state != SessionState::Exited
        })
        .collect::<Vec<_>>();
    if active.is_empty() {
        return Ok(());
    }
    let states = active
        .iter()
        .map(|session| format!("{} ({:?})", session.backend_session_id, session.state))
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "Worktrunk removal refused because the target still has a non-exited Zellij session: {states}. Terminate it, then retry."
    ))
}

fn removal_intent(
    registry: &HostRegistry,
    workspace_id: WorkspaceId,
    expected_target_path: &str,
) -> Result<WorktrunkRemovalIntent, String> {
    if registry
        .worktrunk_removal(workspace_id)
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err(
            "A previous Worktrunk removal has an unknown result; run :worktree list before trying again."
                .to_owned(),
        );
    }
    let local_host_id = registry
        .local_host_id()
        .map_err(|error| error.to_string())?;
    let target = registry
        .workspace(workspace_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "The Worktrunk target workspace is not registered.".to_owned())?;
    validate_registered_target(&target, local_host_id, expected_target_path)?;
    let target_repository_id = target
        .repository
        .as_ref()
        .ok_or_else(|| "Worktrunk target has no registered Git repository identity.".to_owned())?
        .repository_id();
    let canonical_target = canonical_repository(&target.root_path)?;
    let repository_key_value = repository_key(&canonical_target)?;
    let mut candidates = registry
        .snapshot()
        .map_err(|error| error.to_string())?
        .workspaces;
    candidates.sort_by(|left, right| left.root_path.cmp(&right.root_path));
    let survivor = candidates.into_iter().find(|candidate| {
        candidate.id != target.id
            && candidate.host_id == local_host_id
            && candidate
                .repository
                .as_ref()
                .is_some_and(|identity| identity.repository_id() == target_repository_id)
            && canonical_repository(&candidate.root_path)
                .and_then(|path| repository_key(&path))
                .is_ok_and(|key| key == repository_key_value)
    });
    let survivor = survivor.ok_or_else(|| {
        "Register another worktree from this exact Git repository before removing the selected one."
            .to_owned()
    })?;
    registry
        .plan_worktrunk_removal(
            target.id,
            survivor.id,
            expected_target_path,
            repository_key_value,
        )
        .map_err(|error| error.to_string())
}

pub(super) fn validate_registered_target(
    workspace: &WorkspaceRecord,
    local_host_id: crate::core::HostId,
    expected_path: &str,
) -> Result<(), String> {
    if workspace.host_id != local_host_id {
        return Err("Worktrunk workspace belongs to another host.".to_owned());
    }
    if workspace.root_path != expected_path {
        return Err("Worktrunk path does not match the registered workspace ID.".to_owned());
    }
    Ok(())
}

fn validate_git_removal_identity(
    intent: &WorktrunkRemovalIntent,
) -> Result<(PathBuf, PathBuf), String> {
    let surviving = canonical_repository(&intent.surviving_path)?;
    let target = canonical_repository(&intent.target_path)?;
    let surviving_key = repository_key(&surviving)?;
    let target_key = repository_key(&target)?;
    if surviving_key != intent.repository_key || target_key != intent.repository_key {
        return Err(
            "Worktrunk target and surviving workspace are not from the same Git common directory."
                .to_owned(),
        );
    }
    Ok((surviving, target))
}

pub(in crate::host_services) fn repository_key(repository: &Path) -> Result<String, String> {
    let identity = repository_identity(repository);
    if !identity.is_absolute() {
        return Err("Worktrunk Git common-directory identity is not absolute.".to_owned());
    }
    identity
        .to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| "Worktrunk Git common-directory identity must be valid UTF-8.".to_owned())
}
