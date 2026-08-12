use super::agent_events::{AgentRunContext, HostAgentEvents};
use super::tool_runtime::discover_exact_binary;
use crate::core::{
    AgentProcessObservation, CorePaths, HostAgentRun, HostPeriodicRefresh, HostRegistry,
    SessionBackend, SessionState, WorkspaceId,
};
use crate::providers::runtime::AGENT_RUN_ID_ENV;
use crate::transport::LocalTransport;
use crate::zellij::{classify_pane_process, PaneProcessState, ZellijRuntime};
use std::collections::{BTreeMap, BTreeSet};

const MAX_ATTACHED_WORKSPACES: usize = 256;

#[derive(Default)]
struct SessionGroup {
    version: String,
    session: String,
    runs: Vec<HostAgentRun>,
    attached_workspaces: Vec<WorkspaceId>,
}

#[derive(Default)]
struct SessionObservation {
    runs: Vec<(HostAgentRun, Result<AgentProcessObservation, String>)>,
    clients: BTreeMap<WorkspaceId, usize>,
    client_errors: BTreeMap<WorkspaceId, String>,
}

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
    let groups = session_groups(&registry_snapshot, listed_runs, &attached_workspaces);

    let mut observations = Vec::new();
    let mut connected_clients = BTreeMap::new();
    let mut client_count_errors = BTreeMap::new();
    let handles = groups
        .into_values()
        .map(|group| std::thread::spawn(move || observe_session(group)))
        .collect::<Vec<_>>();
    for handle in handles {
        match handle.join() {
            Ok(observed) => {
                observations.extend(observed.runs);
                connected_clients.extend(observed.clients);
                client_count_errors.extend(observed.client_errors);
            }
            Err(_) => return Err("A Zellij observation worker panicked.".to_owned()),
        }
    }
    for workspace_id in attached_workspaces {
        if !client_count_errors.contains_key(&workspace_id) {
            connected_clients.entry(workspace_id).or_insert(0);
        }
    }

    let mut agent_runs = Vec::new();
    let mut agent_snapshots = BTreeMap::new();
    let mut watchable_agent_runs = Vec::new();
    let mut errors = Vec::new();
    for (run, observation) in observations {
        let live_identity = matches!(observation, Ok(AgentProcessObservation::Live));
        let mut current = match observation {
            Ok(observation) => events
                .reconcile_run(context(&run), &run.binding, observation)
                .unwrap_or_else(|error| {
                    errors.push(format!(
                        "Agent run {} reconciliation failed: {error}",
                        run.run_id
                    ));
                    run.clone()
                }),
            Err(error) => {
                errors.push(format!(
                    "Agent run {} observation failed: {error}",
                    run.run_id
                ));
                run
            }
        };
        match events.snapshot(current.run_id) {
            Ok(Some(snapshot)) => {
                let run_id = current.run_id;
                current.snapshot = snapshot.snapshot.clone();
                if snapshot.snapshot.state != crate::agent_status::AgentState::Exited {
                    if live_identity {
                        watchable_agent_runs.push(current.run_id);
                    }
                    agent_runs.push(current);
                }
                agent_snapshots.insert(run_id, snapshot);
            }
            Ok(None) => errors.push(format!(
                "Agent run {} no longer has an authoritative snapshot.",
                current.run_id
            )),
            Err(error) => errors.push(format!(
                "Agent run {} snapshot refresh failed: {error}",
                current.run_id
            )),
        }
    }

    Ok(HostPeriodicRefresh {
        host_id,
        registry: registry_snapshot,
        ports,
        agent_runs,
        agent_snapshots,
        watchable_agent_runs,
        connected_clients,
        client_count_errors,
        errors,
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

fn session_groups(
    snapshot: &crate::core::RegistrySnapshot,
    runs: Vec<HostAgentRun>,
    attached: &[WorkspaceId],
) -> BTreeMap<(String, String), SessionGroup> {
    let mut groups = BTreeMap::new();
    for run in runs {
        let key = (
            run.binding.zellij_version.clone(),
            run.binding.session_name.clone(),
        );
        let group = groups.entry(key).or_insert_with(SessionGroup::default);
        group.version.clone_from(&run.binding.zellij_version);
        group.session.clone_from(&run.binding.session_name);
        group.runs.push(run);
    }
    for workspace_id in attached {
        let session = snapshot
            .sessions
            .iter()
            .filter(|session| {
                session.workspace_id == *workspace_id
                    && session.backend == SessionBackend::Zellij
                    && session.state != SessionState::Exited
            })
            .max_by_key(|session| session.created_at_ms);
        let Some(session) = session else {
            continue;
        };
        let key = (
            session.backend_version.clone(),
            session.backend_session_id.clone(),
        );
        let group = groups.entry(key).or_insert_with(SessionGroup::default);
        group.version.clone_from(&session.backend_version);
        group.session.clone_from(&session.backend_session_id);
        group.attached_workspaces.push(*workspace_id);
    }
    groups
}

fn observe_session(group: SessionGroup) -> SessionObservation {
    let mut result = SessionObservation::default();
    let observed = (|| -> Result<(Vec<crate::zellij::ZellijPane>, usize), String> {
        let binary = discover_exact_binary("Zellij", "zellij", "zellij", &group.version)?;
        let binary = binary
            .to_str()
            .ok_or_else(|| "Zellij binary path is not valid UTF-8.".to_owned())?;
        let zellij = ZellijRuntime::for_version(binary, &group.version)
            .map_err(|error| error.to_string())?;
        let mut transport = LocalTransport;
        let (zellij, session_exists) = zellij
            .resolve_session_namespace(&mut transport, &group.session)
            .map_err(|error| error.to_string())?;
        if !session_exists {
            return Ok((Vec::new(), 0));
        }
        let panes = if group.runs.is_empty() {
            Vec::new()
        } else {
            zellij
                .list_panes(&mut transport, &group.session)
                .map_err(|error| error.to_string())?
        };
        let clients = if group.attached_workspaces.is_empty() {
            0
        } else {
            zellij
                .list_clients(&mut transport, &group.session)
                .map_err(|error| error.to_string())?
                .len()
        };
        Ok((panes, clients))
    })();

    match observed {
        Ok((panes, clients)) => {
            for run in group.runs {
                let observation = if panes.is_empty() {
                    AgentProcessObservation::Missing
                } else {
                    process_observation(classify_pane_process(
                        &panes,
                        run.binding.tab_id,
                        &run.binding.tab_name,
                        &run.binding.zellij_pane_id,
                        &format!("{AGENT_RUN_ID_ENV}={}", run.run_id),
                    ))
                };
                result.runs.push((run, Ok(observation)));
            }
            for workspace_id in group.attached_workspaces {
                result.clients.insert(workspace_id, clients);
            }
        }
        Err(error) => {
            result
                .runs
                .extend(group.runs.into_iter().map(|run| (run, Err(error.clone()))));
            for workspace_id in group.attached_workspaces {
                result.client_errors.insert(workspace_id, error.clone());
            }
        }
    }
    result
}

fn process_observation(state: PaneProcessState) -> AgentProcessObservation {
    match state {
        PaneProcessState::Live => AgentProcessObservation::Live,
        PaneProcessState::Exited { code } => AgentProcessObservation::Exited { exit_code: code },
        PaneProcessState::Missing => AgentProcessObservation::Missing,
        PaneProcessState::UnverifiedIdentity { .. } => AgentProcessObservation::StateUnknown,
    }
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
