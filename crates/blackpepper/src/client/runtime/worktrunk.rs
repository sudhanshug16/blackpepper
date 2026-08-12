use super::{connection, ClientRuntime};
use crate::core::{
    HostServicePayload, RequestOperation, ResponsePayload, WorkspaceId, WorkspaceSetup,
    WorktrunkMutationOutcome,
};
use crate::transport::WORKTRUNK_VERSION;
use crate::worktrunk::{WorktreeList, WorktrunkApprovalToken, WorktrunkProjectCommand};
use std::path::PathBuf;

#[derive(Debug)]
pub(crate) enum WorktreeChange {
    ApprovalRequired {
        command: String,
        approval: WorktrunkApprovalToken,
        unapproved_project_commands: Vec<WorktrunkProjectCommand>,
    },
    Registered {
        workspace_id: WorkspaceId,
        path: PathBuf,
    },
    SetupFailed {
        workspace_id: WorkspaceId,
        path: PathBuf,
        message: String,
    },
    Removed,
    UnknownAfterDisconnect,
}

impl ClientRuntime {
    pub(crate) fn list_worktrees(
        &mut self,
        workspace_id: WorkspaceId,
    ) -> Result<WorktreeList, String> {
        let workspace = self.workspace_record(workspace_id)?;
        self.ensure_worktrunk(workspace.host_id)?;
        let list = match self.host_service(
            workspace.host_id,
            RequestOperation::WorktrunkList {
                workspace_id: workspace.id,
                repository_path: workspace.root_path,
            },
        )? {
            HostServicePayload::Worktrees { list } => list,
            _ => return Err("bp-host returned an unexpected Worktrunk list response.".to_string()),
        };
        // List-time recovery may have removed a durable host workspace. Pull
        // the fresh registry before returning so a lost prior response cannot
        // leave a client-local ghost in the workspace tree.
        connection::refresh_registry(self, workspace.host_id)?;
        Ok(list)
    }

    pub(crate) fn create_worktree(
        &mut self,
        workspace_id: WorkspaceId,
        branch: &str,
        base: Option<&str>,
        approval: Option<WorktrunkApprovalToken>,
    ) -> Result<WorktreeChange, String> {
        let workspace = self.workspace_record(workspace_id)?;
        self.ensure_worktrunk(workspace.host_id)?;
        self.switch_result(
            workspace.host_id,
            RequestOperation::WorktrunkCreate {
                repository_path: workspace.root_path,
                branch: branch.to_string(),
                base: base.map(str::to_string),
                approval,
            },
        )
    }

    pub(crate) fn open_worktree(
        &mut self,
        workspace_id: WorkspaceId,
        selector: &str,
        approval: Option<WorktrunkApprovalToken>,
    ) -> Result<WorktreeChange, String> {
        let workspace = self.workspace_record(workspace_id)?;
        self.ensure_worktrunk(workspace.host_id)?;
        self.switch_result(
            workspace.host_id,
            RequestOperation::WorktrunkSwitch {
                repository_path: workspace.root_path,
                selector: selector.to_string(),
                approval,
            },
        )
    }

    pub(crate) fn remove_worktree(
        &mut self,
        workspace_id: WorkspaceId,
        approval: Option<WorktrunkApprovalToken>,
    ) -> Result<WorktreeChange, String> {
        let mut target = self.workspace_record(workspace_id)?;
        if target.repository.is_none() {
            return Err(
                "This folder is not an identified Git worktree; Blackpepper will not remove it."
                    .to_string(),
            );
        }
        self.ensure_worktrunk(target.host_id)?;
        if approval.is_some() {
            // Preview never disrupts the workspace. Once the exact Worktrunk
            // plan is approved, terminate under the host lifecycle gate and
            // refresh the target before the host-side remover takes that same
            // gate for the mutation itself.
            self.terminate_workspace(workspace_id)?;
            target = self.workspace_record(workspace_id)?;
        }
        let payload = match connection::registry_operation_tracked(
            self,
            target.host_id,
            RequestOperation::WorktrunkRemove {
                workspace_id: target.id,
                target_path: target.root_path,
                approval,
            },
        ) {
            Ok(ResponsePayload::HostService { payload }) => *payload,
            Ok(_) => return Err("bp-host returned an unexpected Worktrunk response.".to_string()),
            Err(error) if error.is_unknown_after_send() => {
                return Ok(WorktreeChange::UnknownAfterDisconnect)
            }
            Err(error) => return Err(error.to_string()),
        };
        match payload {
            HostServicePayload::WorktrunkApprovalRequired {
                command,
                approval,
                unapproved_project_commands,
            } => Ok(WorktreeChange::ApprovalRequired {
                command,
                approval,
                unapproved_project_commands,
            }),
            HostServicePayload::WorktrunkMutation {
                outcome: WorktrunkMutationOutcome::Removed { .. },
            } => {
                // The helper already atomically removed shared host state. This
                // only drops the client's cache (or is an idempotent no-op for
                // the local host, where both registries are the same file).
                self.registry
                    .remove_workspace(target.id)
                    .map_err(|error| error.to_string())?;
                Ok(WorktreeChange::Removed)
            }
            _ => Err("bp-host returned an unexpected Worktrunk removal response.".to_string()),
        }
    }

    fn switch_result(
        &mut self,
        host_id: crate::core::HostId,
        operation: RequestOperation,
    ) -> Result<WorktreeChange, String> {
        let payload = match connection::registry_operation_tracked(self, host_id, operation) {
            Ok(ResponsePayload::HostService { payload }) => *payload,
            Ok(_) => return Err("bp-host returned an unexpected Worktrunk response.".to_string()),
            Err(error) if error.is_unknown_after_send() => {
                return Ok(WorktreeChange::UnknownAfterDisconnect)
            }
            Err(error) => return Err(error.to_string()),
        };
        match payload {
            HostServicePayload::WorktrunkApprovalRequired {
                command,
                approval,
                unapproved_project_commands,
            } => Ok(WorktreeChange::ApprovalRequired {
                command,
                approval,
                unapproved_project_commands,
            }),
            HostServicePayload::WorktrunkMutation {
                outcome: WorktrunkMutationOutcome::Switched { result },
            } => {
                let workspace_id = self.register_workspace(host_id, &result.path)?;
                Ok(WorktreeChange::Registered {
                    workspace_id,
                    path: result.path,
                })
            }
            HostServicePayload::WorktrunkMutation {
                outcome: WorktrunkMutationOutcome::SetupFailed { path, message },
            } => {
                let workspace_id = self.register_workspace(host_id, &path)?;
                let mut workspace = self.workspace_record(workspace_id)?;
                workspace.setup = WorkspaceSetup::Failed {
                    message: message.clone(),
                };
                workspace.touch();
                self.persist_workspace(&workspace)?;
                Ok(WorktreeChange::SetupFailed {
                    workspace_id,
                    path,
                    message,
                })
            }
            _ => Err("bp-host returned an unexpected Worktrunk mutation response.".to_string()),
        }
    }

    fn ensure_worktrunk(&mut self, host_id: crate::core::HostId) -> Result<(), String> {
        self.exact_binary(host_id, "wt", WORKTRUNK_VERSION)
            .map(|_| ())
    }

    fn host_service(
        &mut self,
        host_id: crate::core::HostId,
        operation: RequestOperation,
    ) -> Result<HostServicePayload, String> {
        match connection::registry_operation(self, host_id, operation)? {
            ResponsePayload::HostService { payload } => Ok(*payload),
            _ => Err("bp-host returned an unexpected host-service response.".to_string()),
        }
    }

    fn workspace_record(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<crate::core::WorkspaceRecord, String> {
        self.registry
            .workspace(workspace_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "The selected workspace no longer exists.".to_string())
    }
}
