use super::super::tool_runtime::discover_zellij_binary;
use crate::core::{
    AgentProcessObservation, HostAgentRun, RegistrySnapshot, SessionBackend, SessionState,
    WorkspaceId,
};
use crate::providers::runtime::AGENT_RUN_ID_ENV;
use crate::transport::LocalTransport;
use crate::zellij::{classify_pane_process, PaneProcessState, ZellijPane, ZellijRuntime};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct SessionGroup {
    version: String,
    session: String,
    runs: Vec<HostAgentRun>,
    attached_workspaces: Vec<WorkspaceId>,
}

#[derive(Default)]
pub(super) struct SessionObservation {
    pub(super) runs: Vec<(HostAgentRun, Result<AgentProcessObservation, String>)>,
    pub(super) clients: BTreeMap<WorkspaceId, usize>,
    pub(super) client_errors: BTreeMap<WorkspaceId, String>,
    /// One-based focused tab and total tab count, per attached workspace.
    pub(super) tabs: BTreeMap<WorkspaceId, (u32, u32)>,
}

#[derive(Default)]
struct SessionOverview {
    clients: usize,
    /// One-based focused tab and total tab count.
    tabs: Option<(u32, u32)>,
}

pub(super) fn session_groups(
    snapshot: &RegistrySnapshot,
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

pub(super) fn observe_session(group: SessionGroup) -> SessionObservation {
    let mut transport = LocalTransport;
    let resolved = (|| {
        let binary = discover_zellij_binary(&group.version)?;
        let binary = binary
            .to_str()
            .ok_or_else(|| "Zellij binary path is not valid UTF-8.".to_owned())?;
        let zellij = ZellijRuntime::for_version(binary, &group.version)
            .map_err(|error| error.to_string())?;
        zellij
            .resolve_session_namespace(&mut transport, &group.session)
            .map_err(|error| error.to_string())
    })();

    let (zellij, session_exists) = match resolved {
        Ok(resolved) => resolved,
        Err(error) => {
            return combine_observations(group, Err(error.clone()), Err(error));
        }
    };
    if !session_exists {
        // The exact session probe is authoritative for both consumers. An
        // absent session means every recorded pane is missing and no client is
        // attached; unlike a metadata error, neither result is stale.
        return combine_observations(group, Ok(Vec::new()), Ok(SessionOverview::default()));
    }

    // Agent lifecycle and attached-client overview are independent reads.
    // One unavailable screen snapshot must not discard valid evidence from
    // the other, especially a terminal pane exit that must be persisted.
    let panes = if group.runs.is_empty() {
        Ok(Vec::new())
    } else {
        zellij
            .list_panes(&mut transport, &group.session)
            .map_err(|error| error.to_string())
    };
    let overview = if group.attached_workspaces.is_empty() {
        Ok(SessionOverview::default())
    } else {
        observe_overview(&zellij, &mut transport, &group.session)
    };
    combine_observations(group, panes, overview)
}

fn observe_overview(
    zellij: &ZellijRuntime,
    transport: &mut LocalTransport,
    session: &str,
) -> Result<SessionOverview, String> {
    let clients = zellij
        .list_clients(transport, session)
        .map_err(|error| error.to_string())?
        .len();
    let listed = zellij
        .list_tabs(transport, session)
        .map_err(|error| error.to_string())?;
    // Report the focused tab by its one-based position so the status row
    // reads the way the tab bar does, not by Zellij's internal ID.
    let tabs = listed
        .iter()
        .find(|tab| tab.active)
        .map(|tab| (tab.position as u32 + 1, listed.len() as u32));
    Ok(SessionOverview { clients, tabs })
}

/// Combine already-bounded observations without allowing either consumer to
/// erase valid evidence from the other. Kept pure so the independence
/// contract can be tested without a live Zellij server.
fn combine_observations(
    group: SessionGroup,
    panes: Result<Vec<ZellijPane>, String>,
    overview: Result<SessionOverview, String>,
) -> SessionObservation {
    let mut result = SessionObservation::default();
    match panes {
        Ok(panes) => {
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
        }
        Err(error) => result
            .runs
            .extend(group.runs.into_iter().map(|run| (run, Err(error.clone())))),
    }

    match overview {
        Ok(overview) => {
            for workspace_id in group.attached_workspaces {
                result.clients.insert(workspace_id, overview.clients);
                if let Some(tabs) = overview.tabs {
                    result.tabs.insert(workspace_id, tabs);
                }
            }
        }
        Err(error) => {
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

#[cfg(test)]
#[path = "observation_tests.rs"]
mod tests;
