use super::super::super::runtime::{ClientRuntime, ForwardCleanupBatch, ForwardCleanupOutcome};
use super::super::super::{ClientState, HostConnection};
use crate::core::{HostId, HostPeriodicRefresh};
use std::collections::BTreeSet;
use std::time::Duration;

const BACKGROUND_NOTICE_DURATION: Duration = Duration::from_secs(5);

pub(super) fn forward_cleanup(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    host_id: HostId,
    outcomes: Vec<ForwardCleanupOutcome>,
) {
    let mut failures = Vec::new();
    for outcome in outcomes {
        let Some(index) = cleanup_target_index(&state.forwards, outcome.forward_id, host_id) else {
            continue;
        };
        match outcome.result {
            Ok(()) => {
                let forward = state.forwards.remove(index);
                runtime.confirm_orphan_forward_cleanup(&forward);
            }
            Err(error) => {
                state.forwards[index].status = crate::ports::ForwardStatus::Failed(error.clone());
                failures.push(format!(
                    "{}: {error}",
                    state.forwards[index].remote_endpoint()
                ));
            }
        }
    }
    if !failures.is_empty() {
        state.set_output(format!(
            "Background tunnel cleanup failed on host {host_id}: {}",
            failures.join(" | ")
        ));
    }
}

pub(super) fn refresh(
    state: &mut ClientState,
    runtime: &mut ClientRuntime,
    host_id: HostId,
    result: Result<Box<HostPeriodicRefresh>, String>,
) -> Option<ForwardCleanupBatch> {
    let still_connected = state.connections.get(&host_id).is_some_and(|connection| {
        matches!(
            connection,
            HostConnection::Local | HostConnection::Connected
        )
    });
    if !still_connected {
        return None;
    }
    let refresh = match result {
        Ok(refresh) => refresh,
        Err(error) => {
            state
                .ports
                .insert(host_id, crate::ports::failed_probe(error.clone()));
            mark_host_agents_failed(state, host_id, &error);
            state.set_transient_output(
                format!("Background refresh failed on host {host_id}: {error}"),
                BACKGROUND_NOTICE_DURATION,
            );
            return None;
        }
    };
    if refresh.host_id != host_id {
        state.set_transient_output(
            format!(
                "Background refresh was rejected because host {host_id} reported identity {}.",
                refresh.host_id
            ),
            BACKGROUND_NOTICE_DURATION,
        );
        return None;
    }
    let snapshot = match runtime.apply_periodic_registry(&refresh) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            mark_host_agents_failed(state, host_id, &error);
            state.set_transient_output(
                format!("Registry refresh failed on host {host_id}: {error}"),
                BACKGROUND_NOTICE_DURATION,
            );
            return None;
        }
    };
    state.snapshot = snapshot;
    let cleanup = runtime.prepare_orphan_forward_cleanup(&state.forwards, &state.snapshot, host_id);
    let cleanup_ids = cleanup.forward_ids().collect::<BTreeSet<_>>();
    for forward in &mut state.forwards {
        if cleanup_ids.contains(&forward.id) {
            forward.status = crate::ports::ForwardStatus::Cancelling;
        }
    }
    let mut notices = merge_refresh_state(state, host_id, &refresh);
    let watcher_errors = runtime.ensure_periodic_blocker_watchers(&refresh, state.event_tx.clone());
    notices.extend(watcher_errors);
    if !notices.is_empty() {
        state.set_transient_output(
            format!(
                "Background refresh on host {host_id}: {}",
                notices.join(" | ")
            ),
            BACKGROUND_NOTICE_DURATION,
        );
    }
    (!cleanup.is_empty()).then_some(cleanup)
}

pub(super) fn merge_refresh_state(
    state: &mut ClientState,
    host_id: HostId,
    refresh: &HostPeriodicRefresh,
) -> Vec<String> {
    state.ports.insert(host_id, refresh.ports.clone());
    state.upsert_discovered_agent_runs(host_id, refresh.agent_runs.clone());
    for (run_id, snapshot) in &refresh.agent_snapshots {
        if let Some(run) = state
            .agent_runs
            .get_mut(&snapshot.workspace_id)
            .and_then(|runs| runs.iter_mut().find(|run| run.run_id == *run_id))
        {
            run.apply_host_snapshot(snapshot.clone());
        }
    }
    // Apply observation-pipeline failures after snapshots: the host includes
    // the last known snapshot for diagnostics, but it must not make failed
    // observation look authoritative. The next valid snapshot clears it.
    for (run_id, error) in &refresh.agent_observation_errors {
        if let Some(run) = state
            .agent_runs
            .values_mut()
            .flatten()
            .find(|run| run.run_id == *run_id)
        {
            // Exit is a durable monotonic event. A cleanup or later read
            // warning remains in the transient footer, but must not turn an
            // authoritative exit back into an unknown state.
            if run
                .snapshot
                .as_ref()
                .is_none_or(|snapshot| snapshot.state != crate::agent_status::AgentState::Exited)
            {
                run.mark_snapshot_error(error.clone());
            }
        }
    }
    update_client_counts(state, host_id, refresh);
    update_overviews(state, host_id, refresh);
    for workspace_id in state
        .agent_runs
        .keys()
        .copied()
        .filter(|workspace_id| state.host_for_workspace(*workspace_id) == Some(host_id))
        .collect::<Vec<_>>()
    {
        state.refresh_workspace_status(workspace_id);
    }
    state.rebuild_tree();

    let mut notices = refresh.errors.clone();
    notices.extend(refresh.client_count_errors.values().cloned());
    notices
}

pub(super) fn cleanup_target_index(
    forwards: &[crate::ports::ForwardState],
    forward_id: uuid::Uuid,
    host_id: HostId,
) -> Option<usize> {
    forwards.iter().position(|forward| {
        forward.id == forward_id
            && forward.host_id == host_id
            && forward.status == crate::ports::ForwardStatus::Cancelling
    })
}

pub(super) fn mark_host_agents_failed(state: &mut ClientState, host_id: HostId, error: &str) {
    let workspace_ids = state
        .agent_runs
        .keys()
        .copied()
        .filter(|workspace_id| state.host_for_workspace(*workspace_id) == Some(host_id))
        .collect::<Vec<_>>();
    for workspace_id in workspace_ids {
        if let Some(runs) = state.agent_runs.get_mut(&workspace_id) {
            for run in runs {
                run.mark_snapshot_error(error.to_owned());
            }
        }
        state.refresh_workspace_status(workspace_id);
    }
    state.rebuild_tree();
}

fn update_client_counts(state: &mut ClientState, host_id: HostId, refresh: &HostPeriodicRefresh) {
    let attached = state.terminals.keys().copied().collect::<BTreeSet<_>>();
    state
        .connected_clients
        .retain(|workspace_id, _| attached.contains(workspace_id));
    for (workspace_id, count) in &refresh.connected_clients {
        if attached.contains(workspace_id)
            && state.host_for_workspace(*workspace_id) == Some(host_id)
        {
            state.connected_clients.insert(*workspace_id, *count);
        }
    }
    for workspace_id in refresh.client_count_errors.keys() {
        state.connected_clients.remove(workspace_id);
    }
}

/// Replace this host's repository and tab context wholesale. An older helper
/// reports no overviews at all, which clears this host's entries rather than
/// leaving a branch on screen that nothing is confirming any more.
fn update_overviews(state: &mut ClientState, host_id: HostId, refresh: &HostPeriodicRefresh) {
    let stale = state
        .overviews
        .keys()
        .copied()
        .filter(|workspace_id| state.host_for_workspace(*workspace_id) == Some(host_id))
        .collect::<Vec<_>>();
    for workspace_id in stale {
        state.overviews.remove(&workspace_id);
    }
    for (workspace_id, overview) in &refresh.overviews {
        state.overviews.insert(*workspace_id, overview.clone());
    }
}
