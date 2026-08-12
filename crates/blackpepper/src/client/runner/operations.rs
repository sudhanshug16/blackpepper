//! Apply generation-checked explicit host work after the worker returns.

use super::super::runtime::{
    ClientRuntime, DeferredHostAction, DeferredHostResult, DurableActionQueue,
    HostOperationContext, HostOperationValue, WorktreeChange,
};
use super::super::{actions, control, ClientMode, ClientState};
use crate::core::HostId;

pub(super) fn progress(
    state: &mut ClientState,
    token: uuid::Uuid,
    host_id: HostId,
    message: String,
) {
    let Some((current, label)) = state.host_operations.get_mut(&host_id) else {
        return;
    };
    if *current != token {
        return;
    }
    *label = message.clone();
    state.set_output(message);
}

pub(in crate::client) fn complete(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    token: uuid::Uuid,
    host_id: HostId,
    generation: u64,
) {
    let completed = runtime.finish_host_operation(host_id, generation, token);
    // Disconnect deliberately discards the worker-owned transport and result.
    // Its exact token must still leave the visible operation map, while a
    // stale token must never clear a newer operation for the same host.
    if completed.is_none() {
        if state
            .host_operations
            .get(&host_id)
            .is_some_and(|(current, _)| *current == token)
        {
            state.host_operations.remove(&host_id);
        }
        return;
    }
    let completed = completed.expect("checked host operation completion disappeared");
    debug_assert_eq!(completed.host_id, host_id);
    if state
        .host_operations
        .get(&host_id)
        .is_some_and(|(current, _)| *current == token)
    {
        state.host_operations.remove(&host_id);
    }
    if completed.discarded {
        let worktrunk_unknown = matches!(
            &completed.context,
            HostOperationContext::WorktreeMutation { .. }
        );
        let initial_focus_workspace = match &completed.context {
            HostOperationContext::InitialShellFocus { workspace_id } => Some(*workspace_id),
            _ => None,
        };
        apply_deferred_results(state, completed.deferred_results);
        apply_deferred_results(
            state,
            completed
                .deferred_remaining
                .into_iter()
                .map(|action| match action {
                    DeferredHostAction::MarkDetached { workspace_id } => {
                        DeferredHostResult::Detached {
                            workspace_id,
                            result: Err(
                                "The host disconnected before this state update could be persisted."
                                    .to_owned(),
                            ),
                        }
                    }
                    DeferredHostAction::MarkAgentsUnknown {
                        workspace_id,
                        run_ids,
                    } => DeferredHostResult::AgentsUnknown {
                        workspace_id,
                        results: run_ids
                            .into_iter()
                            .map(|run_id| {
                                (
                                    run_id,
                                    Err("The host disconnected before this state update could be persisted."
                                        .to_owned()),
                                )
                            })
                            .collect(),
                    },
                })
                .collect(),
        );
        if worktrunk_unknown {
            state.set_output(
                "Worktrunk result is Unknown after disconnect; it was not retried. Reconnect and run :worktree list to reconcile.",
            );
        } else if let Some(workspace_id) = initial_focus_workspace {
            resume_attached_workspace(state, workspace_id);
            state.set_output(
                "Initial workspace shell focus was cancelled by disconnect; no focus change was accepted.",
            );
        }
        state.rebuild_tree();
        return;
    }
    match completed.snapshot {
        Ok(snapshot) => state.snapshot = snapshot,
        Err(error) => {
            state.set_output(format!(
                "{} finished, but the local registry could not be refreshed: {error}",
                completed.label
            ));
        }
    }
    let context = completed.context;
    let result = completed.result;
    match result {
        Err(error) => fail_context(state, context, &completed.label, error),
        Ok(value) => apply_value(state, runtime, host_id, context, value, &completed.label),
    }
    apply_deferred_results(state, completed.deferred_results);
    start_remaining_deferred(state, runtime, host_id, completed.deferred_remaining);
    state.rebuild_tree();
}

fn start_remaining_deferred(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    host_id: HostId,
    actions: Vec<DeferredHostAction>,
) {
    if actions.is_empty() {
        return;
    }
    match runtime.queue_durable_actions(
        host_id,
        "Persisting queued terminal state",
        actions,
        state.event_tx.clone(),
    ) {
        Ok(DurableActionQueue::Started { token, label }) => {
            state.host_operations.insert(host_id, (token, label));
        }
        Ok(DurableActionQueue::Queued { .. }) => {}
        Err(error) => state.set_output(format!(
            "Queued terminal state could not be persisted: {error}"
        )),
    }
}

pub(in crate::client) fn apply_deferred_results(
    state: &mut ClientState,
    results: Vec<DeferredHostResult>,
) {
    let mut failures = Vec::new();
    for result in results {
        match result {
            DeferredHostResult::Detached {
                workspace_id,
                result,
            } => {
                if let Err(error) = result {
                    failures.push(format!(
                        "Detached session {workspace_id} could not be recorded: {error}"
                    ));
                }
            }
            DeferredHostResult::AgentsUnknown {
                workspace_id,
                results,
            } => {
                for (run_id, result) in results {
                    match result {
                        Ok(persisted) => {
                            if let Some(run) = state
                                .agent_runs
                                .get_mut(&workspace_id)
                                .and_then(|runs| runs.iter_mut().find(|run| run.run_id == run_id))
                            {
                                run.apply_snapshot(persisted.snapshot);
                            }
                        }
                        Err(error) => {
                            let message = format!(
                                "Ctrl-C status for agent {run_id} could not be persisted: {error}"
                            );
                            if let Some(run) = state
                                .agent_runs
                                .get_mut(&workspace_id)
                                .and_then(|runs| runs.iter_mut().find(|run| run.run_id == run_id))
                            {
                                run.mark_snapshot_error(message.clone());
                            }
                            failures.push(message);
                        }
                    }
                }
                state.refresh_workspace_status(workspace_id);
            }
        }
    }
    if !failures.is_empty() {
        state.set_detail("Terminal state persistence errors", failures.join("\n\n"));
        state.set_output(
            "Terminal input was handled, but durable status needs attention; review the error panel.",
        );
    }
}

fn apply_value(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    host_id: HostId,
    context: HostOperationContext,
    value: HostOperationValue,
    label: &str,
) {
    match (context, value) {
        (
            HostOperationContext::DurableState,
            HostOperationValue::DurableState(results),
        ) => apply_deferred_results(state, results),
        (
            HostOperationContext::SshImportPreview,
            HostOperationValue::SshImportPreview(previews),
        ) => actions::apply_import_preview(state, previews),
        (
            HostOperationContext::AgentSpawn {
                workspace_id,
                provider,
            },
            HostOperationValue::AgentSpawned(spawned),
        ) => actions::apply_spawned(state, workspace_id, provider, spawned),
        (
            HostOperationContext::ServiceStart { workspace_id, name },
            HostOperationValue::ServiceStarted { tab_id },
        ) => {
            state.selected_workspace = Some(workspace_id);
            state.set_output(format!(
                "Service {name} is running in background Zellij tab {tab_id}."
            ));
        }
        (HostOperationContext::WorktreeList { workspace_id }, HostOperationValue::Worktrees(list)) => {
            state.selected_workspace = Some(workspace_id);
            actions::apply_worktree_list(state, list)
        }
        (
            HostOperationContext::WorktreeMutation {
                workspace_id,
                command,
                replaces_forwards,
            },
            HostOperationValue::WorktreeMutation(mut result),
        ) => {
            if replaces_forwards {
                state
                    .forwards
                    .retain(|forward| forward.workspace_id != workspace_id);
                state
                    .forwards
                    .extend(result.forwards.take().unwrap_or_default());
            }
            match result.change {
                Ok(change) => {
                    let removed = matches!(change, WorktreeChange::Removed);
                    actions::apply_worktree_change(
                        state,
                        change,
                        Some((workspace_id, command)),
                        result.session_error,
                    );
                    if removed {
                        if state.active_workspace == Some(workspace_id) {
                            state.active_workspace = None;
                            state.mode = ClientMode::Manage;
                        }
                        state.selected_workspace = state
                            .workspace_ids()
                            .into_iter()
                            .find(|candidate| *candidate != workspace_id);
                    }
                }
                Err(error) => state.set_output(format!("{label} failed: {error}")),
            }
        }
        (
            HostOperationContext::StatusExplain { workspace_id },
            HostOperationValue::AgentDiagnostics { snapshots },
        ) => actions::apply_explain(state, workspace_id, snapshots),
        (
            HostOperationContext::PortList { host_id, all_host },
            HostOperationValue::Ports { snapshot },
        ) => actions::apply_port_list(state, host_id, all_host, snapshot),
        (
            HostOperationContext::ForwardStart { workspace_id },
            HostOperationValue::Forwarded(forward),
        ) if forward.workspace_id == workspace_id => actions::apply_forwarded(state, forward),
        (
            HostOperationContext::ForwardCancel {
                workspace_id,
                forward_id,
            },
            HostOperationValue::ForwardCancelled(forward),
        ) if forward.id == forward_id && forward.workspace_id == workspace_id => {
            actions::apply_cancelled(state, workspace_id, forward_id, forward)
        }
        (
            HostOperationContext::Attach { workspace_id },
            HostOperationValue::Attached {
                workspace_id: returned,
                process,
                provisional_clients,
            },
        ) if workspace_id == returned => {
            match control::apply_attachment(state, workspace_id, process, provisional_clients) {
                Ok(()) => maybe_schedule_initial_shell_focus(
                    state,
                    runtime,
                    host_id,
                    workspace_id,
                    provisional_clients,
                ),
                Err(error) => state.set_output(format!("Workspace attach failed: {error}")),
            }
        }
        (
            HostOperationContext::RegisterAndAttach { host_id, path },
            HostOperationValue::RegisteredAndAttached {
                workspace_id,
                path: returned_path,
                attachment,
            },
        ) if path == returned_path => {
            state.selected_workspace = Some(workspace_id);
            state.selected_host = Some(host_id);
            match attachment {
                Ok((process, clients)) => {
                    match control::apply_attachment(state, workspace_id, process, clients) {
                        Ok(()) => maybe_schedule_initial_shell_focus(
                            state,
                            runtime,
                            host_id,
                            workspace_id,
                            clients,
                        ),
                        Err(error) => state.set_output(format!(
                            "Registered workspace {}, but its terminal could not attach: {error}",
                            path.display()
                        )),
                    }
                }
                Err(error) => state.set_output(format!(
                    "Registered workspace {}, but its persistent shell could not start: {error}",
                    path.display()
                )),
            }
        }
        (
            HostOperationContext::InitialShellFocus { workspace_id },
            HostOperationValue::InitialShellFocused,
        ) => {
            resume_attached_workspace(state, workspace_id);
            state.clear_output();
        }
        (
            HostOperationContext::WorkspaceUngroup { workspace_id },
            HostOperationValue::WorkspaceUngrouped(workspace),
        ) if workspace.id == workspace_id => actions::apply_ungrouped_workspace(state, workspace),
        (HostOperationContext::Terminate { workspace_id }, HostOperationValue::Terminated) => {
            state.terminals.remove(&workspace_id);
            state.set_output("Zellij session terminated; the workspace folder was kept.")
        }
        _ => state.set_output(format!(
            "{label} returned a mismatched result; its host state was retained, but the UI did not apply it."
        )),
    }
}

fn maybe_schedule_initial_shell_focus(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    host_id: HostId,
    workspace_id: crate::core::WorkspaceId,
    provisional_clients: usize,
) {
    // `attach_workspace` returns the count observed before spawning plus this
    // attachment. Only the 0 -> 1 transition needs correction: with no client
    // context, Zellij 0.44.3 leaves the last auto-start tab as next focus.
    if provisional_clients != 1 {
        return;
    }
    let result =
        schedule_initial_shell_focus_with(state, runtime, host_id, workspace_id, move |runtime| {
            runtime.focus_initial_shell_after_first_attach(workspace_id)
        });
    if let Err(error) = result {
        state.set_output(format!(
            "Workspace attached, but initial shell focus could not start: {error}"
        ));
    }
}

fn schedule_initial_shell_focus_with<F>(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    host_id: HostId,
    workspace_id: crate::core::WorkspaceId,
    work: F,
) -> Result<(), String>
where
    F: FnOnce(&mut ClientRuntime) -> Result<(), String> + Send + 'static,
{
    let label = "Selecting initial workspace shell".to_owned();
    let token = runtime.start_host_operation(
        host_id,
        label.clone(),
        HostOperationContext::InitialShellFocus { workspace_id },
        state.event_tx.clone(),
        Box::new(move |runtime| work(runtime).map(|()| HostOperationValue::InitialShellFocused)),
    )?;
    state.host_operations.insert(host_id, (token, label));
    // The reader thread was spawned by `apply_attachment`, but input must not
    // race the bounded focus correction into an auto-start service pane.
    state.mode = ClientMode::Manage;
    state.set_output("Workspace attached; selecting its initial shell…");
    Ok(())
}

fn resume_attached_workspace(state: &mut ClientState, workspace_id: crate::core::WorkspaceId) {
    if state.active_workspace == Some(workspace_id) && state.terminals.contains_key(&workspace_id) {
        state.mode = ClientMode::Work;
    }
}

fn fail_context(
    state: &mut ClientState,
    context: HostOperationContext,
    label: &str,
    error: String,
) {
    if let HostOperationContext::InitialShellFocus { workspace_id } = &context {
        resume_attached_workspace(state, *workspace_id);
    }
    if let HostOperationContext::WorktreeMutation {
        workspace_id,
        replaces_forwards: true,
        ..
    } = &context
    {
        for forward in state
            .forwards
            .iter_mut()
            .filter(|forward| forward.workspace_id == *workspace_id)
        {
            forward.status = crate::ports::ForwardStatus::Failed(format!(
                "Worktrunk operation outcome unavailable: {error}"
            ));
        }
    }
    if let HostOperationContext::ForwardCancel { forward_id, .. } = &context {
        if let Some(forward) = state
            .forwards
            .iter_mut()
            .find(|forward| forward.id == *forward_id)
        {
            forward.status = crate::ports::ForwardStatus::Failed(error.clone());
        }
    }
    state.set_output(format!("{label} failed: {error}"));
}

#[cfg(test)]
#[path = "operations_tests.rs"]
mod tests;
