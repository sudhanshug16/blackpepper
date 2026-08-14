use super::*;
use crate::agent_status::{AgentStatusTracker, NeedsInputCapability, Provider};
use crate::core::{AgentRunBinding, AgentRunId, HostId, PaneId, SessionId};

#[test]
fn exited_and_missing_pane_evidence_survives_overview_failure() {
    let exited = run("terminal_9");
    let missing = run("terminal_10");
    let exited_id = exited.run_id;
    let missing_id = missing.run_id;
    let workspace = WorkspaceId::new();
    let pane = ZellijPane {
        id: 9,
        is_plugin: false,
        tab_id: exited.binding.tab_id,
        tab_name: exited.binding.tab_name.clone(),
        exited: true,
        exit_status: Some(130),
        is_held: false,
        terminal_command: Some(format!("codex {AGENT_RUN_ID_ENV}={exited_id}")),
        pane_command: Some("codex".to_owned()),
    };
    let group = SessionGroup {
        runs: vec![exited, missing],
        attached_workspaces: vec![workspace],
        ..SessionGroup::default()
    };

    let observed = combine_observations(
        group,
        Ok(vec![pane]),
        Err("invalid tab JSON: EOF while parsing a value".to_owned()),
    );
    let by_run = observed
        .runs
        .into_iter()
        .map(|(run, result)| (run.run_id, result))
        .collect::<BTreeMap<_, _>>();

    assert_eq!(
        by_run.get(&exited_id),
        Some(&Ok(AgentProcessObservation::Exited {
            exit_code: Some(130)
        }))
    );
    assert_eq!(
        by_run.get(&missing_id),
        Some(&Ok(AgentProcessObservation::Missing))
    );
    assert!(observed.clients.is_empty());
    assert!(observed.tabs.is_empty());
    assert!(observed
        .client_errors
        .get(&workspace)
        .is_some_and(|error| error.contains("invalid tab JSON")));
}

#[test]
fn valid_overview_survives_pane_failure() {
    let run = run("terminal_9");
    let run_id = run.run_id;
    let workspace = WorkspaceId::new();
    let group = SessionGroup {
        runs: vec![run],
        attached_workspaces: vec![workspace],
        ..SessionGroup::default()
    };

    let observed = combine_observations(
        group,
        Err("invalid pane JSON: EOF while parsing a value".to_owned()),
        Ok(SessionOverview {
            clients: 2,
            tabs: Some((2, 3)),
        }),
    );

    assert!(matches!(
        observed.runs.as_slice(),
        [(observed_run, Err(error))]
            if observed_run.run_id == run_id && error.contains("invalid pane JSON")
    ));
    assert_eq!(observed.clients.get(&workspace), Some(&2));
    assert_eq!(observed.tabs.get(&workspace), Some(&(2, 3)));
    assert!(observed.client_errors.is_empty());
}

fn run(zellij_pane_id: &str) -> HostAgentRun {
    let run_id = AgentRunId::new();
    HostAgentRun {
        host_id: HostId::new(),
        workspace_id: WorkspaceId::new(),
        run_id,
        pane_id: PaneId::new(),
        provider: Provider::Codex,
        binding: AgentRunBinding {
            session_id: SessionId::new(),
            session_name: "repo-main".to_owned(),
            zellij_version: "0.44.3-blackpepper.2".to_owned(),
            tab_id: 4,
            tab_name: "agent".to_owned(),
            zellij_pane_id: zellij_pane_id.to_owned(),
        },
        snapshot: AgentStatusTracker::new(
            run_id,
            Provider::Codex,
            NeedsInputCapability::BlockerOverlay,
        )
        .snapshot(),
    }
}
