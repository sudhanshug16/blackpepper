#[path = "worktrunk_reconcile/removal.rs"]
mod removal;

use super::super::worktrunk_lock::RepositoryLock;
use super::{canonical_repository, execute, require_success, WorktrunkExecutor};
use crate::core::{HostRegistry, HostServicePayload, WorkspaceId};
use crate::worktrunk::{WorktreeList, Worktrunk, WorktrunkApprovalToken};
use std::path::{Path, PathBuf};

pub(super) use removal::repository_key;
use removal::validate_registered_target;

pub(super) fn list(
    executor: &WorktrunkExecutor,
    registry: &HostRegistry,
    workspace_id: WorkspaceId,
    repository_path: &str,
) -> Result<HostServicePayload, String> {
    let (repository, repository_key) = list_repository(registry, workspace_id, repository_path)?;
    let lock = RepositoryLock::acquire(&executor.lock_dir, &repository)?;
    let spec = Worktrunk::new(executor.binary()?).list(&repository);
    let output = execute(&lock, &spec)?;
    require_success(&spec, &output)?;
    let json = std::str::from_utf8(&output.stdout)
        .map_err(|_| "Worktrunk list output was not valid UTF-8.".to_owned())?;
    let list = WorktreeList::parse(json)?;
    reconcile_removal_intents(registry, &repository, &repository_key, &list)?;
    Ok(HostServicePayload::Worktrees { list })
}

pub(super) fn remove(
    executor: &WorktrunkExecutor,
    registry: &HostRegistry,
    workspace_id: WorkspaceId,
    target_path: &str,
    approval: Option<&WorktrunkApprovalToken>,
) -> Result<HostServicePayload, String> {
    removal::remove(executor, registry, workspace_id, target_path, approval)
}

fn list_repository(
    registry: &HostRegistry,
    workspace_id: WorkspaceId,
    expected_path: &str,
) -> Result<(PathBuf, String), String> {
    let local_host_id = registry
        .local_host_id()
        .map_err(|error| error.to_string())?;
    if let Some(workspace) = registry
        .workspace(workspace_id)
        .map_err(|error| error.to_string())?
    {
        validate_registered_target(&workspace, local_host_id, expected_path)?;
        if workspace.repository.is_none() {
            return Err(
                "The selected workspace has no registered Git repository identity.".to_owned(),
            );
        }
        if let Ok(repository) = canonical_repository(expected_path) {
            let key = repository_key(&repository)?;
            return Ok((repository, key));
        }
    }
    recovery_repository(registry, workspace_id, expected_path, local_host_id)
}

fn recovery_repository(
    registry: &HostRegistry,
    workspace_id: WorkspaceId,
    expected_path: &str,
    local_host_id: crate::core::HostId,
) -> Result<(PathBuf, String), String> {
    let intent = registry
        .worktrunk_removal(workspace_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| {
            "The selected workspace path is unavailable and has no pending Worktrunk removal to reconcile."
                .to_owned()
        })?;
    if intent.host_id != local_host_id || intent.target_path != expected_path {
        return Err("Pending Worktrunk removal does not match the selected workspace.".to_owned());
    }
    let survivor = registry
        .workspace(intent.surviving_workspace_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "The recorded surviving Worktrunk workspace is unavailable.".to_owned())?;
    if survivor.host_id != local_host_id
        || survivor.root_path != intent.surviving_path
        || survivor
            .repository
            .as_ref()
            .is_none_or(|identity| identity.repository_id() != intent.repository_id)
    {
        return Err("The recorded surviving Worktrunk workspace changed.".to_owned());
    }
    let repository = canonical_repository(&survivor.root_path)?;
    let key = repository_key(&repository)?;
    if key != intent.repository_key {
        return Err("The recorded Worktrunk repository identity changed.".to_owned());
    }
    Ok((repository, key))
}

fn reconcile_removal_intents(
    registry: &HostRegistry,
    repository: &Path,
    repository_key: &str,
    list: &WorktreeList,
) -> Result<(), String> {
    let intents = registry
        .worktrunk_removals_for_repository(repository_key)
        .map_err(|error| error.to_string())?;
    if intents.is_empty() {
        return Ok(());
    }
    let paths = list
        .items
        .iter()
        .filter_map(|item| item.worktree.as_ref().map(|worktree| &worktree.path))
        .collect::<Vec<_>>();
    if paths.iter().any(|path| !path.is_absolute())
        || !paths.iter().any(|path| path.as_path() == repository)
    {
        return Err(
            "Worktrunk list was not authoritative for the surviving repository; removal recovery was left unchanged."
                .to_owned(),
        );
    }
    for intent in intents {
        if paths
            .iter()
            .any(|path| path.as_path() == Path::new(&intent.target_path))
        {
            registry
                .cancel_worktrunk_removal(&intent)
                .map_err(|error| error.to_string())?;
        } else {
            registry
                .finish_worktrunk_removal(&intent)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}
