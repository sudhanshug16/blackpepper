use super::super::command::ClientCommand;
use super::super::state::PendingWorktrunkApproval;
use super::super::{ClientMode, ClientState};
use crate::client::runtime::{
    ClientRuntime, HostOperationContext, HostOperationValue, WorktreeChange, WorktreeMutationResult,
};

pub(super) fn list(state: &mut ClientState, runtime: &mut ClientRuntime) -> Result<(), String> {
    let workspace_id = super::selected_workspace(state)?;
    let host_id = state
        .host_for_workspace(workspace_id)
        .ok_or_else(|| "The selected workspace host is unavailable.".to_owned())?;
    let label = "Listing Worktrunk worktrees".to_owned();
    let token = runtime.start_host_operation(
        host_id,
        label.clone(),
        HostOperationContext::WorktreeList { workspace_id },
        state.event_tx.clone(),
        Box::new(move |runtime| {
            runtime
                .list_worktrees(workspace_id)
                .map(HostOperationValue::Worktrees)
        }),
    )?;
    state
        .host_operations
        .insert(host_id, (token, label.clone()));
    state.set_output(format!("{label}… Press Esc in Manage mode to cancel."));
    Ok(())
}

pub(in crate::client) fn apply_list(state: &mut ClientState, list: crate::worktrunk::WorktreeList) {
    let lines = list
        .items
        .iter()
        .map(|item| {
            let branch = item.branch.as_deref().unwrap_or("detached");
            let path = item
                .worktree
                .as_ref()
                .map(|worktree| worktree.path.display().to_string())
                .unwrap_or_else(|| "not checked out".to_string());
            format!("{branch}: {path}")
        })
        .collect::<Vec<_>>();
    let message = if lines.is_empty() {
        "Worktrunk returned no worktrees.".to_string()
    } else {
        let count = lines.len();
        state.set_detail("Worktrees", lines.join("\n"));
        format!("Listed {count} Worktrunk item(s). Esc closes the list.")
    };
    state.set_output(message);
}

pub(super) fn create(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    branch: String,
    base: Option<String>,
) -> Result<(), String> {
    let workspace_id = super::selected_workspace(state)?;
    let command = ClientCommand::WorktreeCreate {
        branch: branch.clone(),
        base: base.clone(),
    };
    schedule_mutation(state, runtime, workspace_id, command, None)
}

pub(super) fn open(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    selector: String,
) -> Result<(), String> {
    let command = ClientCommand::WorktreeOpen {
        selector: selector.clone(),
    };
    let workspace_id = super::selected_workspace(state)?;
    schedule_mutation(state, runtime, workspace_id, command, None)
}

pub(super) fn remove(state: &mut ClientState, runtime: &mut ClientRuntime) -> Result<(), String> {
    let workspace_id = super::selected_workspace(state)?;
    schedule_mutation(
        state,
        runtime,
        workspace_id,
        ClientCommand::WorktreeRemove,
        None,
    )
}

pub(super) fn approve(state: &mut ClientState, runtime: &mut ClientRuntime) -> Result<(), String> {
    let pending = state
        .pending_approval
        .take()
        .ok_or_else(|| "There is no pending Worktrunk command to approve.".to_string())?;
    if let Err(error) = schedule_mutation(
        state,
        runtime,
        pending.workspace_id,
        pending.command.clone(),
        Some(pending.approval.clone()),
    ) {
        state.pending_approval = Some(pending);
        return Err(error);
    }
    Ok(())
}

pub(in crate::client) fn apply_change(
    state: &mut ClientState,
    change: WorktreeChange,
    pending: Option<(crate::core::WorkspaceId, ClientCommand)>,
    session_error: Option<String>,
) {
    match change {
        WorktreeChange::ApprovalRequired {
            command,
            approval,
            unapproved_project_commands,
        } => {
            let Some((workspace_id, pending_command)) = pending else {
                state.set_output("Worktrunk's approval plan changed; rerun the original command.");
                return;
            };
            state.pending_approval = Some(PendingWorktrunkApproval {
                workspace_id,
                command: pending_command,
                approval,
                review: approval_review(&command, &unapproved_project_commands),
            });
            state.approval_scroll = 0;
            state.set_output(
                "Review the complete Worktrunk mutation and project commands above, then run :approve. Any change invalidates this approval.",
            );
        }
        WorktreeChange::Registered { workspace_id, path } => {
            state.selected_workspace = Some(workspace_id);
            match session_error {
                None => state.set_output(format!(
                    "Registered worktree {} with a persistent shell.",
                    path.display()
                )),
                Some(error) => state.set_output(format!(
                    "Registered worktree {}, but its shell could not start: {error}",
                    path.display()
                )),
            }
        }
        WorktreeChange::SetupFailed {
            workspace_id,
            path,
            message,
        } => {
            state.selected_workspace = Some(workspace_id);
            state.set_output(format!(
                "Worktree {} exists but setup failed: {message}",
                path.display()
            ));
        }
        WorktreeChange::UnknownAfterDisconnect => state.set_output(
            "Unknown after disconnect; Blackpepper did not retry. Reconnect and run :worktree list.",
        ),
        WorktreeChange::Removed => {
            state.set_output("Worktree removed through Worktrunk without force flags.");
        }
    }
}

fn approval_review(
    command: &str,
    projects: &[crate::worktrunk::WorktrunkProjectCommand],
) -> String {
    let mut sections = vec![format!("mutation\n{command}")];
    if projects.is_empty() {
        sections.push("unapproved project hooks\nnone".to_string());
    } else {
        let hooks = projects
            .iter()
            .map(|project| {
                let name = project
                    .name
                    .as_deref()
                    .map(|name| format!(" / {name}"))
                    .unwrap_or_default();
                format!("{}{name}: {}", project.phase, project.template)
            })
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("unapproved project hooks\n{hooks}"));
    }
    sections.push(
        "approval binds to this exact Worktrunk command and project hook plan.\n:approve  run · esc dismiss · ↑↓ scroll\nAny change invalidates this approval; Blackpepper never adds force or hook-skipping flags."
            .to_string(),
    );
    sections.join("\n\n")
}

fn schedule_mutation(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    workspace_id: crate::core::WorkspaceId,
    command: ClientCommand,
    approval: Option<crate::worktrunk::WorktrunkApprovalToken>,
) -> Result<(), String> {
    let host_id = state
        .host_for_workspace(workspace_id)
        .ok_or_else(|| "The selected workspace host is unavailable.".to_owned())?;
    let approved_removal = approval.is_some() && matches!(command, ClientCommand::WorktreeRemove);
    let operation_forwards = state
        .forwards
        .iter()
        .filter(|forward| approved_removal && forward.workspace_id == workspace_id)
        .cloned()
        .collect::<Vec<_>>();
    let context_command = command.clone();
    let label = if approval.is_some() {
        "Applying approved Worktrunk mutation"
    } else {
        "Preparing Worktrunk mutation for review"
    }
    .to_owned();
    let token = runtime.start_host_operation(
        host_id,
        label.clone(),
        HostOperationContext::WorktreeMutation {
            workspace_id,
            command: context_command,
            replaces_forwards: approved_removal,
        },
        state.event_tx.clone(),
        Box::new(move |runtime| {
            let mut forwards = operation_forwards;
            let change = match command {
                ClientCommand::WorktreeCreate { branch, base } => {
                    runtime.create_worktree(workspace_id, &branch, base.as_deref(), approval)
                }
                ClientCommand::WorktreeOpen { selector } => {
                    runtime.open_worktree(workspace_id, &selector, approval)
                }
                ClientCommand::WorktreeRemove => {
                    let cancellation = if approved_removal {
                        runtime.cancel_workspace_forwards(&mut forwards, workspace_id)
                    } else {
                        Ok(0)
                    };
                    cancellation.and_then(|_| runtime.remove_worktree(workspace_id, approval))
                }
                _ => Err("The background command is not a Worktrunk mutation.".to_owned()),
            };
            let session_error = match &change {
                Ok(WorktreeChange::Registered { workspace_id, .. }) => {
                    runtime.restore_workspace(*workspace_id).err()
                }
                _ => None,
            };
            Ok(HostOperationValue::WorktreeMutation(
                WorktreeMutationResult {
                    change,
                    forwards: approved_removal.then_some(forwards),
                    session_error,
                },
            ))
        }),
    );
    let token = token?;
    if approved_removal {
        for forward in state
            .forwards
            .iter_mut()
            .filter(|forward| forward.workspace_id == workspace_id)
        {
            forward.status = crate::ports::ForwardStatus::Cancelling;
        }
        state.terminals.remove(&workspace_id);
        state.connected_clients.remove(&workspace_id);
        if state.active_workspace == Some(workspace_id) {
            state.active_workspace = None;
            state.mode = ClientMode::Manage;
        }
    }
    state
        .host_operations
        .insert(host_id, (token, label.clone()));
    state.set_output(format!("{label}… Press Esc in Manage mode to cancel; Blackpepper will never retry an uncertain mutation."));
    Ok(())
}

#[cfg(test)]
mod tests;
