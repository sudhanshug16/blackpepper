use std::collections::BTreeMap;
use std::sync::mpsc;

use crate::agent_status::{
    AgentSnapshot, AgentState, AgentStatusTracker, IntegrationHealth, NeedsInputCapability,
    Provider,
};
use crate::client::runtime::ClientRuntime;
use crate::client::{ClientState, DisplayStatus, HostConnection};
use crate::core::{
    AgentRunBinding, AgentRunId, HostAgentRun, HostAgentSnapshot, HostId, HostPeriodicRefresh,
    PaneId, RegistrySnapshot, SessionId, WorkspaceId, WorkspaceRecord,
};

use super::super::apply::{merge_refresh_state, refresh as apply_refresh};

#[test]
fn background_refresh_error_temporarily_overlays_without_replacing_user_output() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = ClientRuntime::test_fixture(root.path());
    let host_id = runtime.local_host_id();
    let (events, _receiver) = mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        runtime.snapshot().unwrap(),
        events,
    );
    state.connections.insert(host_id, HostConnection::Local);
    state.set_output("User command result remains available.");

    assert!(apply_refresh(
        &mut state,
        &mut runtime,
        host_id,
        Err("invalid tab JSON: EOF while parsing a value".to_owned()),
    )
    .is_none());

    assert_eq!(
        state.output.as_deref(),
        Some("User command result remains available.")
    );
    assert!(state
        .visible_output()
        .unwrap()
        .contains("Background refresh failed"));
}

#[test]
fn observation_error_is_non_authoritative_and_the_next_snapshot_clears_it() {
    let root = tempfile::tempdir().unwrap();
    let runtime = ClientRuntime::test_fixture(root.path());
    let host_id = runtime.local_host_id();
    let workspace = WorkspaceRecord::new(host_id, "/tmp/metadata-retry-workspace");
    runtime.registry.upsert_workspace(&workspace).unwrap();
    let (events, _receiver) = mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        runtime.snapshot().unwrap(),
        events,
    );
    let run = host_run(host_id, workspace.id);
    let run_id = run.run_id;

    let failed = refresh(
        host_id,
        vec![run.clone()],
        BTreeMap::from([(run_id, "invalid pane JSON: EOF".to_owned())]),
    );
    merge_refresh_state(&mut state, host_id, &failed);

    let displayed = &state.agent_runs[&workspace.id][0];
    assert_eq!(displayed.display_status(), DisplayStatus::Unknown);
    assert_eq!(
        displayed.snapshot_error.as_deref(),
        Some("invalid pane JSON: EOF")
    );

    let recovered = refresh(host_id, vec![run], BTreeMap::new());
    merge_refresh_state(&mut state, host_id, &recovered);

    let displayed = &state.agent_runs[&workspace.id][0];
    assert_eq!(displayed.display_status(), DisplayStatus::Working);
    assert!(displayed.snapshot_error.is_none());
}

#[test]
fn authoritative_exit_is_not_replaced_by_a_same_refresh_cleanup_error() {
    let root = tempfile::tempdir().unwrap();
    let runtime = ClientRuntime::test_fixture(root.path());
    let host_id = runtime.local_host_id();
    let workspace = WorkspaceRecord::new(host_id, "/tmp/metadata-exit-workspace");
    runtime.registry.upsert_workspace(&workspace).unwrap();
    let (events, _receiver) = mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        runtime.snapshot().unwrap(),
        events,
    );
    let run = host_run(host_id, workspace.id);
    let run_id = run.run_id;
    merge_refresh_state(
        &mut state,
        host_id,
        &refresh(host_id, vec![run], BTreeMap::new()),
    );

    let mut exited = snapshot(run_id);
    exited.state = AgentState::Exited;
    let explain = AgentStatusTracker::from_snapshot(exited.clone()).explain();
    let mut exited_refresh = refresh(
        host_id,
        Vec::new(),
        BTreeMap::from([(
            run_id,
            "exit persisted, but managed integration cleanup failed".to_owned(),
        )]),
    );
    exited_refresh.agent_snapshots.insert(
        run_id,
        HostAgentSnapshot {
            host_id,
            workspace_id: workspace.id,
            pane_id: None,
            snapshot: exited,
            explain,
        },
    );

    merge_refresh_state(&mut state, host_id, &exited_refresh);

    let displayed = &state.agent_runs[&workspace.id][0];
    assert_eq!(displayed.display_status(), DisplayStatus::Exited);
    assert!(displayed.snapshot_error.is_none());
}

fn refresh(
    host_id: HostId,
    agent_runs: Vec<HostAgentRun>,
    agent_observation_errors: BTreeMap<AgentRunId, String>,
) -> HostPeriodicRefresh {
    HostPeriodicRefresh {
        host_id,
        registry: RegistrySnapshot::default(),
        ports: crate::ports::failed_probe("not observed by this unit fixture"),
        agent_runs,
        agent_snapshots: BTreeMap::new(),
        agent_observation_errors,
        watchable_agent_runs: Vec::new(),
        connected_clients: BTreeMap::new(),
        client_count_errors: BTreeMap::new(),
        errors: Vec::new(),
        overviews: BTreeMap::new(),
    }
}

fn host_run(host_id: HostId, workspace_id: WorkspaceId) -> HostAgentRun {
    let run_id = AgentRunId::new();
    HostAgentRun {
        host_id,
        workspace_id,
        run_id,
        pane_id: PaneId::new(),
        provider: Provider::Codex,
        binding: AgentRunBinding {
            session_id: SessionId::new(),
            session_name: "bp-test".to_owned(),
            zellij_version: crate::transport::ZELLIJ_VERSION.to_owned(),
            tab_id: 2,
            tab_name: "agent-test".to_owned(),
            zellij_pane_id: "terminal_4".to_owned(),
        },
        snapshot: snapshot(run_id),
    }
}

fn snapshot(run_id: AgentRunId) -> AgentSnapshot {
    AgentSnapshot {
        run_id,
        provider: Provider::Codex,
        state: AgentState::Working,
        revision: 1,
        completion_revision: 0,
        seen_completion_revision: 0,
        last_event_sequence: Some(1),
        last_event_at_ms: Some(1),
        integration_health: IntegrationHealth::Healthy {
            integration_version: Some(1),
        },
        needs_input_capability: NeedsInputCapability::ProviderEventsWithOverlay,
        completion_suppressed: false,
    }
}
