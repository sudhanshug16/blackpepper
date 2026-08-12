use super::super::state::AgentRunView;
use super::super::ClientState;
use crate::agent_status::Provider;
use crate::client::runtime::{ClientRuntime, HostOperationContext, HostOperationValue};

pub(super) fn explain(state: &mut ClientState, runtime: &mut ClientRuntime) -> Result<(), String> {
    let workspace_id = super::selected_workspace(state)?;
    let host_id = state
        .host_for_workspace(workspace_id)
        .ok_or_else(|| "The selected workspace host is unavailable.".to_owned())?;
    let run_ids = state
        .agent_runs
        .get(&workspace_id)
        .into_iter()
        .flatten()
        .map(|run| run.run_id)
        .collect::<Vec<_>>();
    if run_ids.is_empty() {
        render_explain(state, workspace_id);
        return Ok(());
    }
    let label = "Refreshing redacted agent status evidence".to_owned();
    let token = runtime.start_host_operation(
        host_id,
        label.clone(),
        HostOperationContext::StatusExplain { workspace_id },
        state.event_tx.clone(),
        Box::new(move |runtime| {
            let snapshots = run_ids
                .into_iter()
                .map(|run_id| (run_id, runtime.agent_snapshot(host_id, run_id)))
                .collect();
            Ok(HostOperationValue::AgentDiagnostics { snapshots })
        }),
    )?;
    state
        .host_operations
        .insert(host_id, (token, label.clone()));
    state.set_output(format!("{label}… Press Esc in Manage mode to cancel."));
    Ok(())
}

pub(in crate::client) fn apply_explain(
    state: &mut ClientState,
    workspace_id: crate::core::WorkspaceId,
    snapshots: Vec<(
        crate::core::AgentRunId,
        Result<Option<crate::core::HostAgentSnapshot>, String>,
    )>,
) {
    for (run_id, refreshed) in snapshots {
        let Some(run) = state
            .agent_runs
            .get_mut(&workspace_id)
            .and_then(|runs| runs.iter_mut().find(|run| run.run_id == run_id))
        else {
            continue;
        };
        match refreshed {
            Ok(Some(snapshot)) => run.apply_host_snapshot(snapshot),
            Ok(None) => {
                run.mark_snapshot_error(
                    "the host returned no snapshot for this registered run".to_owned(),
                );
            }
            Err(error) => {
                run.mark_snapshot_error(error);
            }
        }
    }
    render_explain(state, workspace_id);
}

fn render_explain(state: &mut ClientState, workspace_id: crate::core::WorkspaceId) {
    let details = state
        .agent_runs
        .get(&workspace_id)
        .into_iter()
        .flatten()
        .map(format_run)
        .collect::<Vec<_>>();
    let message = if details.is_empty() {
        "No agent run is registered for this workspace.".to_string()
    } else {
        let count = details.len();
        let mut body = details.join("\n\n");
        body.push_str(
            "\n\nDiagnostics retain no prompt, response, command, tool content, or terminal text.",
        );
        state.set_detail("Agent status evidence", body);
        format!("Showing {count} agent run diagnostic(s). Esc closes the details.")
    };
    state.set_output(message);
    state.refresh_workspace_status(workspace_id);
}

fn format_run(run: &AgentRunView) -> String {
    let blocker = run
        .blocker
        .as_ref()
        .map(|blocker| {
            format!(
                ", blocker zellij_viewport manifest {} rule {} confidence {:?} observed {:?}",
                blocker.manifest_version,
                blocker.rule_id,
                blocker.confidence,
                run.blocker_observed_at_ms
            )
        })
        .unwrap_or_default();
    let authority = run
        .explain
        .as_ref()
        .map(|explain| {
            format!(
                ", authority {:?}, last event {:?}",
                explain.authority, explain.last_event_kind
            )
        })
        .unwrap_or_else(|| ", authority unavailable".to_owned());
    let failure = run
        .snapshot_error
        .as_ref()
        .map(|error| format!(", refresh failure {error}"))
        .unwrap_or_default();
    let detail = match &run.snapshot {
        Some(snapshot) => format!(
            "{} {:?}, health {:?}, needs_input {}, sequence {:?}, observed {:?}{authority}{failure}{blocker}",
            run.provider,
            run.display_status(),
            snapshot.integration_health,
            run.displayed_needs_input_capability(),
            snapshot.last_event_sequence,
            snapshot.last_event_at_ms
        ),
        None => format!(
            "{} {:?}, needs_input {}{authority}{failure}{blocker}",
            run.provider,
            run.display_status(),
            run.displayed_needs_input_capability()
        ),
    };
    format!("run {} pane {}: {detail}", run.run_id, run.pane_id)
}

pub(super) fn spawn(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    provider: Provider,
) -> Result<(), String> {
    let workspace_id = super::selected_workspace(state)?;
    let host_id = state
        .host_for_workspace(workspace_id)
        .ok_or_else(|| "The selected workspace host is unavailable.".to_owned())?;
    let operation_events = state.event_tx.clone();
    let blocker_events = state.event_tx.clone();
    let label = format!("Starting {provider} (preflight and health handshake)");
    let token = runtime.start_host_operation(
        host_id,
        label.clone(),
        HostOperationContext::AgentSpawn {
            workspace_id,
            provider,
        },
        operation_events,
        Box::new(move |runtime| {
            runtime
                .spawn_agent(workspace_id, provider, blocker_events)
                .map(HostOperationValue::AgentSpawned)
        }),
    )?;
    state
        .host_operations
        .insert(host_id, (token, label.clone()));
    state.set_output(format!("{label}… Press Esc in Manage mode to cancel."));
    Ok(())
}

pub(in crate::client) fn apply_spawned(
    state: &mut ClientState,
    workspace_id: crate::core::WorkspaceId,
    provider: Provider,
    spawned: crate::client::runtime::SpawnedAgent,
) {
    state
        .agent_runs
        .entry(workspace_id)
        .or_default()
        .push(AgentRunView {
            run_id: spawned.run_id,
            pane_id: spawned.pane_id,
            tab_id: spawned.tab_id,
            provider,
            zellij_pane_id: spawned.zellij_pane_id,
            needs_input_capability: spawned.capability.to_string(),
            snapshot: None,
            explain: None,
            snapshot_error: None,
            seen_completion_revision: 0,
            blocker: None,
            blocker_watcher_instance: None,
            blocker_sequence: 0,
            blocker_observed_at_ms: None,
            interrupted_after_sequence: None,
        });
    state
        .statuses
        .insert(workspace_id, super::super::DisplayStatus::Unknown);
    state.set_output(format!(
        "Spawned {provider} in background tab {} (run {}; needs_input: {}). Use native Zellij tab selection to open it.",
        spawned.tab_id, spawned.run_id, spawned.capability
    ));
}

pub(super) fn start_service(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    name: &str,
) -> Result<(), String> {
    let workspace_id = super::selected_workspace(state)?;
    let host_id = state
        .host_for_workspace(workspace_id)
        .ok_or_else(|| "The selected workspace host is unavailable.".to_owned())?;
    let service_name = name.to_owned();
    let worker_name = service_name.clone();
    let label = format!("Starting service {service_name}");
    let token = runtime.start_host_operation(
        host_id,
        label.clone(),
        HostOperationContext::ServiceStart {
            workspace_id,
            name: service_name,
        },
        state.event_tx.clone(),
        Box::new(move |runtime| {
            runtime
                .start_named_service(workspace_id, &worker_name)
                .map(|tab_id| HostOperationValue::ServiceStarted { tab_id })
        }),
    )?;
    state
        .host_operations
        .insert(host_id, (token, label.clone()));
    state.set_output(format!("{label}… Press Esc in Manage mode to cancel."));
    Ok(())
}
