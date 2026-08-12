use super::process::{run_bounded_guarded, BoundedOutput};
use super::tool_runtime::discover_exact_binary;
#[cfg(test)]
use super::tool_runtime::validate_exact_binary;
use super::worktrunk_approval::{authorize, ApprovalDecision};
use super::worktrunk_lock::RepositoryLock;
use crate::core::{
    CorePaths, HostRegistry, HostServicePayload, WorkspaceId, WorktrunkMutationOutcome,
};
use crate::transport::WORKTRUNK_VERSION;
use crate::worktrunk::{
    CommandSpec, SwitchResult, WorktreeList, Worktrunk, WorktrunkApprovalToken,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[path = "worktrunk_reconcile.rs"]
mod reconcile;

pub(super) struct WorktrunkExecutor {
    binary: Result<PathBuf, String>,
    lock_dir: PathBuf,
    paths: CorePaths,
}

impl WorktrunkExecutor {
    pub fn discover(paths: &CorePaths) -> Self {
        Self {
            binary: discover_binary(),
            lock_dir: paths.repository_lock_dir(),
            paths: paths.clone(),
        }
    }

    #[cfg(test)]
    pub fn with_binary(paths: &CorePaths, binary: PathBuf) -> Self {
        Self {
            binary: validate_binary(binary),
            lock_dir: paths.repository_lock_dir(),
            paths: paths.clone(),
        }
    }

    pub fn list(
        &self,
        registry: &HostRegistry,
        workspace_id: WorkspaceId,
        repository_path: &str,
    ) -> Result<HostServicePayload, String> {
        reconcile::list(self, registry, workspace_id, repository_path)
    }

    pub fn create(
        &self,
        repository_path: &str,
        branch: &str,
        base: Option<&str>,
        approval: Option<&WorktrunkApprovalToken>,
    ) -> Result<HostServicePayload, String> {
        let repository = canonical_repository(repository_path)?;
        let spec = Worktrunk::new(self.binary()?).create(&repository, branch, base)?;
        self.switch_mutation(repository, spec, approval)
    }

    pub fn switch(
        &self,
        repository_path: &str,
        selector: &str,
        approval: Option<&WorktrunkApprovalToken>,
    ) -> Result<HostServicePayload, String> {
        let repository = canonical_repository(repository_path)?;
        let spec = Worktrunk::new(self.binary()?).switch(&repository, selector)?;
        self.switch_mutation(repository, spec, approval)
    }

    pub fn remove(
        &self,
        registry: &HostRegistry,
        workspace_id: WorkspaceId,
        target_path: &str,
        approval: Option<&WorktrunkApprovalToken>,
    ) -> Result<HostServicePayload, String> {
        reconcile::remove(self, registry, workspace_id, target_path, approval)
    }

    fn switch_mutation(
        &self,
        repository: PathBuf,
        spec: CommandSpec,
        approval: Option<&WorktrunkApprovalToken>,
    ) -> Result<HostServicePayload, String> {
        let lock = match authorize(self.binary()?, &self.lock_dir, &repository, &spec, approval)? {
            ApprovalDecision::Required(payload) => return Ok(*payload),
            ApprovalDecision::Authorized(lock) => lock,
        };
        // Worktrunk 0.72.0 does not emit its JSON switch result when a
        // pre-start hook fails after creating the worktree. Capture the
        // physical topology under this same repository lock so that one
        // unambiguous new folder can still be registered as setup-failed.
        let paths_before = worktree_paths(&lock, self.binary()?, &repository).ok();
        let output = execute(&lock, &spec)?;
        reject_declined_commands(&output)?;
        let json = std::str::from_utf8(&output.stdout).unwrap_or_default();
        if output.status.success() {
            let result = SwitchResult::parse(json)?;
            return Ok(HostServicePayload::WorktrunkMutation {
                outcome: WorktrunkMutationOutcome::Switched { result },
            });
        }
        if let Ok(result) = SwitchResult::parse(json) {
            if result.path.is_dir() {
                return Ok(HostServicePayload::WorktrunkMutation {
                    outcome: WorktrunkMutationOutcome::SetupFailed {
                        path: result.path,
                        message: failure_message(&spec, &output),
                    },
                });
            }
        }
        if let Some(paths_before) = paths_before {
            if let Ok(paths_after) = worktree_paths(&lock, self.binary()?, &repository) {
                let mut created = paths_after
                    .difference(&paths_before)
                    .filter(|path| path.is_absolute() && path.is_dir());
                if let Some(path) = created.next().cloned() {
                    if created.next().is_none() {
                        return Ok(HostServicePayload::WorktrunkMutation {
                            outcome: WorktrunkMutationOutcome::SetupFailed {
                                path,
                                message: failure_message(&spec, &output),
                            },
                        });
                    }
                }
            }
        }
        Err(failure_message(&spec, &output))
    }

    fn binary(&self) -> Result<&Path, String> {
        self.binary.as_deref().map_err(Clone::clone)
    }
}

fn worktree_paths(
    lock: &RepositoryLock,
    binary: &Path,
    repository: &Path,
) -> Result<BTreeSet<PathBuf>, String> {
    let spec = Worktrunk::new(binary).list(repository);
    let output = execute(lock, &spec)?;
    require_success(&spec, &output)?;
    let json = std::str::from_utf8(&output.stdout)
        .map_err(|_| "Worktrunk list output was not valid UTF-8.".to_owned())?;
    Ok(WorktreeList::parse(json)?
        .items
        .into_iter()
        .filter_map(|item| item.worktree.map(|worktree| worktree.path))
        .collect())
}

fn discover_binary() -> Result<PathBuf, String> {
    discover_exact_binary("Worktrunk", "wt", "worktrunk", WORKTRUNK_VERSION)
}

#[cfg(test)]
fn validate_binary(binary: PathBuf) -> Result<PathBuf, String> {
    validate_exact_binary(binary, "Worktrunk", WORKTRUNK_VERSION)
}

pub(super) fn execute(lock: &RepositoryLock, spec: &CommandSpec) -> Result<BoundedOutput, String> {
    if spec.contains_forbidden_force_flag() {
        return Err("Unsafe Worktrunk force option was blocked.".to_owned());
    }
    run_bounded_guarded(lock, spec.program.as_os_str(), &spec.args)
        .map_err(|error| format!("Could not run Worktrunk: {error}"))
}

pub(super) fn require_success(spec: &CommandSpec, output: &BoundedOutput) -> Result<(), String> {
    if output.status.success() {
        Ok(())
    } else {
        Err(failure_message(spec, output))
    }
}

fn reject_declined_commands(output: &BoundedOutput) -> Result<(), String> {
    let declined = [&output.stdout, &output.stderr].iter().any(|bytes| {
        String::from_utf8_lossy(bytes)
            .to_ascii_lowercase()
            .contains("commands declined")
    });
    if declined {
        Err(
            "Worktrunk declined project commands; the mutation result is not accepted as success."
                .to_owned(),
        )
    } else {
        Ok(())
    }
}

fn failure_message(spec: &CommandSpec, output: &BoundedOutput) -> String {
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail.trim();
    let truncated = if output.truncated {
        " Output was truncated."
    } else {
        ""
    };
    if detail.is_empty() {
        format!("Worktrunk command failed: {}.{truncated}", spec.display())
    } else {
        format!("Worktrunk command failed: {detail}.{truncated}")
    }
}

fn canonical_repository(value: &str) -> Result<PathBuf, String> {
    let path = absolute_target(value)?;
    let canonical = fs::canonicalize(&path)
        .map_err(|error| format!("Could not open repository {}: {error}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!("Repository {} is not a directory.", path.display()));
    }
    Ok(canonical)
}

fn absolute_target(value: &str) -> Result<PathBuf, String> {
    if value.is_empty() || value.len() > 16 * 1024 || value.contains('\0') {
        return Err("Worktrunk path is empty, invalid, or too long.".to_owned());
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err("Worktrunk paths must be absolute.".to_owned());
    }
    Ok(path)
}

#[cfg(test)]
#[path = "worktrunk_exec_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "worktrunk_removal_tests.rs"]
mod removal_tests;

#[cfg(all(test, unix))]
#[path = "worktrunk_supervision_tests.rs"]
mod supervision_tests;
