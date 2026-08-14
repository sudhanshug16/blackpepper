use super::agent_events::{AgentRunContext, HostAgentEvents};
use crate::core::{
    AgentProcessObservation, CorePaths, HostAgentRun, HostPeriodicRefresh, HostRegistry,
    WorkspaceId,
};
use std::collections::{BTreeMap, BTreeSet};

mod observation;

const MAX_ATTACHED_WORKSPACES: usize = 256;

pub(super) fn refresh(
    paths: &CorePaths,
    registry: &HostRegistry,
    attached_workspaces: Vec<WorkspaceId>,
) -> Result<HostPeriodicRefresh, String> {
    validate_attached(&attached_workspaces)?;
    let registry_snapshot = registry.snapshot().map_err(|error| error.to_string())?;
    let host_id = registry
        .local_host_id()
        .map_err(|error| error.to_string())?;
    let ports = super::ports::discover(registry);
    let mut events = HostAgentEvents::open(paths)?;
    let listed_runs = events.list_runs(None)?;
    let groups = observation::session_groups(&registry_snapshot, listed_runs, &attached_workspaces);

    let mut observations = Vec::new();
    let mut connected_clients = BTreeMap::new();
    let mut client_count_errors = BTreeMap::new();
    let mut session_tabs = BTreeMap::new();
    let handles = groups
        .into_values()
        .map(|group| std::thread::spawn(move || observation::observe_session(group)))
        .collect::<Vec<_>>();
    for handle in handles {
        match handle.join() {
            Ok(observed) => {
                observations.extend(observed.runs);
                connected_clients.extend(observed.clients);
                client_count_errors.extend(observed.client_errors);
                session_tabs.extend(observed.tabs);
            }
            Err(_) => return Err("A Zellij observation worker panicked.".to_owned()),
        }
    }
    let mut overviews = BTreeMap::new();
    for workspace_id in attached_workspaces {
        if !client_count_errors.contains_key(&workspace_id) {
            connected_clients.entry(workspace_id).or_insert(0);
        }
        // The header names the checkout the client is looking at, so only the
        // attached workspaces are worth a git call each refresh.
        let Some(record) = registry_snapshot
            .workspaces
            .iter()
            .find(|record| record.id == workspace_id)
        else {
            continue;
        };
        let mut overview = super::repo_status::overview(&record.root_path);
        if let Some((active, count)) = session_tabs.get(&workspace_id).copied() {
            overview.active_tab = Some(active);
            overview.tab_count = Some(count);
        }
        overviews.insert(workspace_id, overview);
    }

    let mut agent_runs = Vec::new();
    let mut agent_snapshots = BTreeMap::new();
    let mut agent_observation_errors = BTreeMap::new();
    let mut watchable_agent_runs = Vec::new();
    let mut errors = Vec::new();
    for (run, observation) in observations {
        let (mut current, live_identity) = match observation {
            Ok(observation) => {
                let live_identity = observation == AgentProcessObservation::Live;
                match events.reconcile_run(context(&run), &run.binding, observation) {
                    Ok(current) => (current, live_identity),
                    Err(error) => {
                        agent_observation_errors.insert(run.run_id, error.clone());
                        errors.push(format!(
                            "Agent run {} reconciliation failed: {error}",
                            run.run_id
                        ));
                        (run.clone(), false)
                    }
                }
            }
            Err(error) => {
                agent_observation_errors.insert(run.run_id, error.clone());
                errors.push(format!(
                    "Agent run {} observation failed: {error}",
                    run.run_id
                ));
                (run, false)
            }
        };
        match events.snapshot(current.run_id) {
            Ok(Some(snapshot)) => {
                let run_id = current.run_id;
                current.snapshot = snapshot.snapshot.clone();
                if snapshot.snapshot.state == crate::agent_status::AgentState::Exited {
                    // Exit is durable and monotonic even when its best-effort
                    // managed-asset cleanup returned an error. Keep that
                    // cleanup warning visible without making exit uncertain.
                    agent_observation_errors.remove(&run_id);
                } else {
                    if live_identity {
                        watchable_agent_runs.push(current.run_id);
                    }
                    agent_runs.push(current);
                }
                agent_snapshots.insert(run_id, snapshot);
            }
            Ok(None) => {
                let error = "no longer has an authoritative snapshot.".to_owned();
                agent_observation_errors.insert(current.run_id, error.clone());
                errors.push(format!("Agent run {} {error}", current.run_id));
            }
            Err(error) => {
                agent_observation_errors.insert(current.run_id, error.clone());
                errors.push(format!(
                    "Agent run {} snapshot refresh failed: {error}",
                    current.run_id
                ));
            }
        }
    }

    Ok(HostPeriodicRefresh {
        host_id,
        registry: registry_snapshot,
        ports,
        agent_runs,
        agent_snapshots,
        agent_observation_errors,
        watchable_agent_runs,
        connected_clients,
        client_count_errors,
        errors,
        overviews,
    })
}

fn validate_attached(workspaces: &[WorkspaceId]) -> Result<(), String> {
    if workspaces.len() > MAX_ATTACHED_WORKSPACES {
        return Err(format!(
            "Periodic refresh accepts at most {MAX_ATTACHED_WORKSPACES} attached workspaces."
        ));
    }
    if workspaces.iter().copied().collect::<BTreeSet<_>>().len() != workspaces.len() {
        return Err("Periodic refresh received duplicate workspace IDs.".to_owned());
    }
    Ok(())
}

fn context(run: &HostAgentRun) -> AgentRunContext {
    AgentRunContext {
        host_id: run.host_id,
        workspace_id: run.workspace_id,
        run_id: run.run_id,
        pane_id: Some(run.pane_id),
        provider: run.provider,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_attached_workspaces_are_rejected() {
        let workspace = WorkspaceId::new();
        assert!(validate_attached(&[workspace, workspace])
            .unwrap_err()
            .contains("duplicate"));
    }
}
