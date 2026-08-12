use super::agent_events::healthy_event;
use super::*;
use crate::agent_status::{AgentEventKind, AgentEventSource, AgentState, Provider};
use crate::core::{
    serve_json_lines_with_extension, AgentProcessObservation, AgentRunBinding, AgentRunId,
    HelperRequest, HelperResponse, HostRegistry, HostServicePayload, PaneId, RequestOperation,
    ResponsePayload, ResponseResult, SessionBackend, SessionRecord, SessionState, WorkspaceId,
    WorkspaceRecord, PROTOCOL_VERSION,
};
use std::io::Cursor;
use std::sync::{Arc, Barrier};

fn setup() -> (tempfile::TempDir, CorePaths, HostRegistry) {
    let root = tempfile::tempdir().unwrap();
    let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
    paths.prepare().unwrap();
    let mut registry = HostRegistry::open(paths.registry_path()).unwrap();
    registry.ensure_local_host("test-host").unwrap();
    (root, paths, registry)
}

fn bound_run(
    registry: &HostRegistry,
    paths: &CorePaths,
    provider: Provider,
) -> (AgentRunContext, AgentRunBinding) {
    let host_id = registry.local_host_id().unwrap();
    let run_id = AgentRunId::new();
    // Keep the fixture under setup()'s TempDir so its guard removes the
    // workspace along with the test registry instead of leaking /tmp entries.
    let workspace_folder = paths.state_dir().join(format!("workspace-{run_id}"));
    std::fs::create_dir(&workspace_folder).unwrap();
    let workspace = WorkspaceRecord::new(host_id, workspace_folder.to_string_lossy());
    registry.upsert_workspace(&workspace).unwrap();
    let mut session = SessionRecord::new(
        workspace.id,
        SessionBackend::Zellij,
        crate::zellij::PINNED_VERSION,
        format!("bp-{}", workspace.id),
    );
    session.state = SessionState::Running;
    registry.upsert_session(&session).unwrap();
    let context = AgentRunContext {
        host_id,
        workspace_id: workspace.id,
        run_id,
        pane_id: Some(PaneId::new()),
        provider,
    };
    let binding = AgentRunBinding {
        session_id: session.id,
        session_name: session.backend_session_id,
        zellij_version: session.backend_version,
        tab_id: 7,
        tab_name: format!("agent-{}", context.run_id),
        zellij_pane_id: "terminal_42".to_owned(),
    };
    let mut events = HostAgentEvents::open(paths).unwrap();
    events.register_run(registry, context).unwrap();
    events.bind_run(registry, context, &binding).unwrap();
    (context, binding)
}

#[test]
fn concurrent_transient_event_store_initialization_is_serialized() {
    let root = tempfile::tempdir().unwrap();
    let paths = Arc::new(CorePaths::from_roots(
        root.path().join("state"),
        root.path().join("run"),
    ));
    paths.prepare().unwrap();
    let barrier = Arc::new(Barrier::new(4));
    let handles = (0..4)
        .map(|_| {
            let paths = Arc::clone(&paths);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                HostAgentEvents::open(&paths).map(|_| ())
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap().unwrap();
    }
}

#[test]
fn pre_binding_context_schema_migrates_in_place() {
    let root = tempfile::tempdir().unwrap();
    let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
    paths.prepare().unwrap();
    let path = paths.agent_events_path();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE agent_run_context (
               run_id TEXT PRIMARY KEY NOT NULL,
               host_id TEXT NOT NULL,
               workspace_id TEXT NOT NULL,
               pane_id TEXT,
               provider TEXT NOT NULL,
               active INTEGER NOT NULL CHECK(active IN (0, 1)),
               created_at_ms INTEGER NOT NULL
             );",
        )
        .unwrap();
    drop(connection);

    drop(HostAgentEvents::open(&paths).unwrap());

    let connection = rusqlite::Connection::open(path).unwrap();
    let mut statement = connection
        .prepare("PRAGMA table_info(agent_run_context)")
        .unwrap();
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for required in [
        "session_id",
        "session_name",
        "zellij_version",
        "tab_id",
        "tab_name",
        "zellij_pane_id",
        "bound_at_ms",
        "deactivated_at_ms",
    ] {
        assert!(columns.iter().any(|column| column == required));
    }
}

#[test]
fn concurrent_hooks_commit_one_monotonic_sequence_per_run() {
    let (_root, paths, registry) = setup();
    let host_id = registry.local_host_id().unwrap();
    let workspace_folder = tempfile::tempdir().unwrap();
    let workspace = WorkspaceRecord::new(host_id, workspace_folder.path().to_string_lossy());
    registry.upsert_workspace(&workspace).unwrap();
    let context = AgentRunContext {
        host_id,
        workspace_id: workspace.id,
        run_id: AgentRunId::new(),
        pane_id: Some(PaneId::new()),
        provider: Provider::Codex,
    };
    let mut events = HostAgentEvents::open(&paths).unwrap();
    events.register_run(&registry, context).unwrap();
    drop(events);

    let paths = Arc::new(paths);
    let registry_path = Arc::new(paths.registry_path());
    let barrier = Arc::new(Barrier::new(4));
    let handles = (0..4)
        .map(|_| {
            let paths = Arc::clone(&paths);
            let registry_path = Arc::clone(&registry_path);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let _registry = HostRegistry::open(registry_path.as_ref()).unwrap();
                let mut events = HostAgentEvents::open(&paths).unwrap();
                barrier.wait();
                events.append(context, AgentEventKind::Working).unwrap();
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }
    let events = HostAgentEvents::open(&paths).unwrap();
    let updates = events.follow(context.run_id, 0, 10).unwrap();
    assert_eq!(updates.len(), 4);
    assert_eq!(
        updates
            .iter()
            .map(|update| update.update.event.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
}

#[test]
fn opencode_heartbeats_store_only_health_edges_and_recover_after_expiry() {
    let (_root, paths, registry) = setup();
    let (context, _) = bound_run(&registry, &paths, Provider::OpenCode);
    let mut events = HostAgentEvents::open(&paths).unwrap();

    events
        .record_opencode_delivery_at(context, &[healthy_event()], 0, 1_000)
        .unwrap();
    assert!(events
        .snapshot_at(context.run_id, 1_000)
        .unwrap()
        .unwrap()
        .snapshot
        .integration_health
        .is_healthy());
    assert_eq!(events.follow(context.run_id, 0, 10).unwrap().len(), 1);

    // A pulse updates one compact row without adding an event.
    events
        .record_opencode_delivery_at(context, &[], 0, 2_000)
        .unwrap();
    assert_eq!(events.follow(context.run_id, 0, 10).unwrap().len(), 1);
    let connection = rusqlite::Connection::open(paths.agent_events_path()).unwrap();
    let freshness = connection
        .query_row(
            "SELECT COUNT(*), MAX(last_seen_at_ms) FROM agent_integration_freshness",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .unwrap();
    assert_eq!(freshness, (1, 2_000));
    drop(connection);

    let stale_at = 2_000 + crate::providers::runtime::OPENCODE_HEALTH_STALE_AFTER_MS + 1;
    let stale = events
        .snapshot_at(context.run_id, stale_at)
        .unwrap()
        .unwrap();
    assert_eq!(
        stale.snapshot.integration_health,
        crate::agent_status::IntegrationHealth::Stale
    );
    assert_eq!(stale.snapshot.state, AgentState::Unknown);
    let updates = events.follow(context.run_id, 0, 10).unwrap();
    assert_eq!(updates.len(), 2);
    assert_eq!(
        updates[1].update.event.source,
        AgentEventSource::IntegrationSupervisor
    );
    assert!(matches!(
        updates[1].update.event.kind,
        AgentEventKind::IntegrationHealthChanged {
            health: crate::agent_status::IntegrationHealth::Stale
        }
    ));
    // Repeated stale reads do not grow the event log.
    events.snapshot_at(context.run_id, stale_at + 1).unwrap();
    assert_eq!(events.follow(context.run_id, 0, 10).unwrap().len(), 2);

    events
        .record_opencode_delivery_at(context, &[], 0, stale_at + 2)
        .unwrap();
    let recovered = events
        .snapshot_at(context.run_id, stale_at + 2)
        .unwrap()
        .unwrap();
    assert!(recovered.snapshot.integration_health.is_healthy());
    let updates = events.follow(context.run_id, 0, 10).unwrap();
    assert_eq!(updates.len(), 3);
    assert_eq!(
        updates[2].update.event.source,
        AgentEventSource::ProviderIntegration
    );
}

#[test]
fn failed_opencode_semantic_delivery_cannot_advance_freshness_or_health() {
    let (_root, paths, registry) = setup();
    let (context, _) = bound_run(&registry, &paths, Provider::OpenCode);
    let mut events = HostAgentEvents::open(&paths).unwrap();

    events
        .record_opencode_delivery_at(context, &[healthy_event()], 0, 1_000)
        .unwrap();
    let connection = rusqlite::Connection::open(paths.agent_events_path()).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_one_opencode_event
             BEFORE INSERT ON agent_status_events
             BEGIN SELECT RAISE(FAIL, 'injected event failure'); END;",
        )
        .unwrap();

    // The permission edge is rolled back by SQLite. Its cursor and timestamp
    // must not advance even though bp-host itself will still exit zero.
    assert!(events
        .record_opencode_delivery_at(context, &[AgentEventKind::NeedsInput], 1, 2_000,)
        .is_err());
    connection
        .execute_batch("DROP TRIGGER fail_one_opencode_event;")
        .unwrap();
    let freshness = connection
        .query_row(
            "SELECT last_seen_at_ms, semantic_sequence, delivery_gap
             FROM agent_integration_freshness",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(freshness, (1_000, 0, false));
    drop(connection);

    // The next heartbeat reports the attempted permission cursor. Because it
    // does not match the last committed semantic cursor, authority becomes
    // stale and cannot skip ahead on a later event.
    assert!(events
        .record_opencode_delivery_at(context, &[], 1, 2_100)
        .is_err());
    let stale = events.snapshot_at(context.run_id, 2_100).unwrap().unwrap();
    assert_eq!(
        stale.snapshot.integration_health,
        crate::agent_status::IntegrationHealth::Stale
    );
    assert_eq!(stale.snapshot.state, AgentState::Unknown);
    assert!(events
        .record_opencode_delivery_at(context, &[AgentEventKind::Working], 2, 2_200)
        .is_err());
    let still_stale = events.snapshot_at(context.run_id, 2_200).unwrap().unwrap();
    assert_eq!(still_stale.snapshot.state, AgentState::Unknown);
    let connection = rusqlite::Connection::open(paths.agent_events_path()).unwrap();
    let cursor = connection
        .query_row(
            "SELECT semantic_sequence, delivery_gap FROM agent_integration_freshness",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?)),
        )
        .unwrap();
    assert_eq!(cursor, (0, true));
}

#[test]
fn ignored_opencode_native_event_does_not_create_a_false_cursor_gap() {
    let (_root, paths, registry) = setup();
    let (context, _) = bound_run(&registry, &paths, Provider::OpenCode);
    let mut events = HostAgentEvents::open(&paths).unwrap();
    events
        .record_opencode_delivery_at(context, &[healthy_event()], 0, 1_000)
        .unwrap();

    // The managed plugin filters unknown native events before incrementing
    // its semantic cursor. The following heartbeat therefore still reports
    // cursor zero and refreshes normally.
    events
        .record_opencode_delivery_at(context, &[], 0, 2_000)
        .unwrap();
    let snapshot = events.snapshot_at(context.run_id, 2_000).unwrap().unwrap();
    assert!(snapshot.snapshot.integration_health.is_healthy());
    assert_eq!(events.follow(context.run_id, 0, 10).unwrap().len(), 1);
}

#[test]
fn duplicate_opencode_semantic_cursor_fails_closed_without_overwrite() {
    let (_root, paths, registry) = setup();
    let (context, _) = bound_run(&registry, &paths, Provider::OpenCode);
    let mut events = HostAgentEvents::open(&paths).unwrap();
    events
        .record_opencode_delivery_at(context, &[healthy_event()], 0, 1_000)
        .unwrap();
    events
        .record_opencode_delivery_at(context, &[AgentEventKind::Working], 1, 1_100)
        .unwrap();

    assert!(events
        .record_opencode_delivery_at(context, &[AgentEventKind::NeedsInput], 1, 1_200)
        .is_err());
    let snapshot = events.snapshot_at(context.run_id, 1_200).unwrap().unwrap();
    assert_eq!(snapshot.snapshot.state, AgentState::Unknown);
    assert_eq!(
        snapshot.snapshot.integration_health,
        crate::agent_status::IntegrationHealth::Stale
    );
    let updates = events.follow(context.run_id, 0, 10).unwrap();
    assert_eq!(updates.len(), 3);
    assert!(updates
        .iter()
        .all(|update| update.update.event.kind != AgentEventKind::NeedsInput));
}

#[test]
fn future_opencode_heartbeat_timestamp_fails_closed_after_clock_rollback() {
    let (_root, paths, registry) = setup();
    let (context, _) = bound_run(&registry, &paths, Provider::OpenCode);
    let mut events = HostAgentEvents::open(&paths).unwrap();
    events
        .record_opencode_delivery_at(context, &[healthy_event()], 0, 10_000)
        .unwrap();

    let snapshot = events.snapshot_at(context.run_id, 9_999).unwrap().unwrap();
    assert_eq!(
        snapshot.snapshot.integration_health,
        crate::agent_status::IntegrationHealth::Stale
    );
}

#[test]
fn bound_run_and_snapshot_survive_helper_restart_without_provider_relaunch() {
    let (_root, paths, registry) = setup();
    let (context, binding) = bound_run(&registry, &paths, Provider::Claude);
    let mut events = HostAgentEvents::open(&paths).unwrap();
    events.append(context, AgentEventKind::Working).unwrap();
    drop(events);

    let mut restarted = HostAgentEvents::open(&paths).unwrap();
    let runs = restarted.list_runs(Some(context.workspace_id)).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, context.run_id);
    assert_eq!(runs[0].binding, binding);
    assert_eq!(runs[0].snapshot.state, AgentState::Working);
    assert_eq!(runs[0].snapshot.last_event_sequence, Some(1));
}

#[test]
fn abandoned_unbound_run_is_deactivated_instead_of_rediscovered() {
    let (_root, paths, registry) = setup();
    let host_id = registry.local_host_id().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let workspace = WorkspaceRecord::new(host_id, folder.path().to_string_lossy());
    registry.upsert_workspace(&workspace).unwrap();
    let context = AgentRunContext {
        host_id,
        workspace_id: workspace.id,
        run_id: AgentRunId::new(),
        pane_id: Some(PaneId::new()),
        provider: Provider::Claude,
    };
    let mut events = HostAgentEvents::open(&paths).unwrap();
    events.register_run(&registry, context).unwrap();
    drop(events);
    let integration_dir = paths.state_dir().join("integrations");
    std::fs::create_dir(&integration_dir).unwrap();
    let abandoned = integration_dir.join(format!("claude-{}.json", context.run_id));
    let neighbor = integration_dir.join("claude-user-file.json");
    std::fs::write(&abandoned, b"managed").unwrap();
    std::fs::write(&neighbor, b"user").unwrap();
    let connection = rusqlite::Connection::open(paths.agent_events_path()).unwrap();
    connection
        .execute(
            "UPDATE agent_run_context SET created_at_ms = 0 WHERE run_id = ?1",
            [context.run_id.to_string()],
        )
        .unwrap();
    drop(connection);

    let mut restarted = HostAgentEvents::open(&paths).unwrap();
    assert!(restarted.list_runs(None).unwrap().is_empty());
    assert!(restarted.context(context.run_id).unwrap().is_none());
    assert!(!abandoned.exists());
    assert_eq!(std::fs::read(neighbor).unwrap(), b"user");
}

#[test]
fn abandoned_asset_cleanup_failure_cannot_poison_later_recovery_scans() {
    let (_root, paths, registry) = setup();
    let host_id = registry.local_host_id().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let workspace = WorkspaceRecord::new(host_id, folder.path().to_string_lossy());
    registry.upsert_workspace(&workspace).unwrap();
    let context = AgentRunContext {
        host_id,
        workspace_id: workspace.id,
        run_id: AgentRunId::new(),
        pane_id: Some(PaneId::new()),
        provider: Provider::OpenCode,
    };
    let mut events = HostAgentEvents::open(&paths).unwrap();
    events.register_run(&registry, context).unwrap();
    drop(events);

    let integration_dir = paths.state_dir().join("integrations");
    std::fs::create_dir(&integration_dir).unwrap();
    let undeletable = integration_dir.join(format!("opencode-{}.js", context.run_id));
    std::fs::create_dir(&undeletable).unwrap();
    let connection = rusqlite::Connection::open(paths.agent_events_path()).unwrap();
    connection
        .execute(
            "UPDATE agent_run_context SET created_at_ms = 0 WHERE run_id = ?1",
            [context.run_id.to_string()],
        )
        .unwrap();
    drop(connection);

    let mut restarted = HostAgentEvents::open(&paths).unwrap();
    let error = restarted.list_runs(None).unwrap_err();
    assert!(error.contains("deactivated"));
    assert!(restarted.context(context.run_id).unwrap().is_none());
    assert!(restarted.list_runs(None).unwrap().is_empty());
    assert!(undeletable.is_dir());
}

#[test]
fn exact_binding_is_required_and_missing_pane_exits_without_revival() {
    let (_root, paths, registry) = setup();
    let (context, binding) = bound_run(&registry, &paths, Provider::Codex);
    let mut events = HostAgentEvents::open(&paths).unwrap();
    events.append(context, AgentEventKind::Working).unwrap();

    let mut reused = binding.clone();
    reused.tab_name = "agent-reused-after-reboot".to_owned();
    assert!(events
        .reconcile_run(context, &reused, AgentProcessObservation::Live)
        .is_err());
    assert_eq!(events.list_runs(None).unwrap().len(), 1);

    let exited = events
        .reconcile_run(context, &binding, AgentProcessObservation::Missing)
        .unwrap();
    assert_eq!(exited.snapshot.state, AgentState::Exited);
    assert_eq!(exited.snapshot.last_event_sequence, Some(2));
    assert!(events.list_runs(None).unwrap().is_empty());
    assert!(events
        .reconcile_run(context, &binding, AgentProcessObservation::Live)
        .is_err());
    assert!(events.register_run(&registry, context).is_err());
    assert!(events.append(context, AgentEventKind::Working).is_err());
}

#[test]
fn supervisor_unknown_and_exit_are_monotonic_and_exit_deactivates() {
    let (_root, paths, registry) = setup();
    let (context, binding) = bound_run(&registry, &paths, Provider::OpenCode);
    let mut events = HostAgentEvents::open(&paths).unwrap();
    events.append(context, AgentEventKind::Working).unwrap();

    let unknown = events
        .reconcile_run(context, &binding, AgentProcessObservation::StateUnknown)
        .unwrap();
    assert_eq!(unknown.snapshot.state, AgentState::Unknown);
    assert_eq!(unknown.snapshot.last_event_sequence, Some(2));
    assert_eq!(events.list_runs(None).unwrap().len(), 1);

    let integration_dir = paths.state_dir().join("integrations");
    std::fs::create_dir(&integration_dir).unwrap();
    let managed_asset = integration_dir.join(format!("opencode-{}.js", context.run_id));
    std::fs::write(&managed_asset, b"managed").unwrap();

    let exited = events
        .reconcile_run(
            context,
            &binding,
            AgentProcessObservation::Exited {
                exit_code: Some(17),
            },
        )
        .unwrap();
    assert_eq!(exited.snapshot.state, AgentState::Exited);
    let updates = events.follow(context.run_id, 0, 10).unwrap();
    assert_eq!(updates.len(), 3);
    assert_eq!(
        updates[1].update.event.source,
        AgentEventSource::ProcessSupervisor
    );
    assert_eq!(updates[1].update.event.kind, AgentEventKind::StateUnknown);
    assert_eq!(
        updates[2].update.event.kind,
        AgentEventKind::Exited {
            exit_code: Some(17)
        }
    );
    assert!(events.list_runs(None).unwrap().is_empty());
    assert!(!managed_asset.exists());
}

#[test]
fn asset_cleanup_failure_cannot_keep_a_dead_run_active() {
    let (_root, paths, registry) = setup();
    let (context, binding) = bound_run(&registry, &paths, Provider::Claude);
    let integration_dir = paths.state_dir().join("integrations");
    std::fs::create_dir(&integration_dir).unwrap();
    let managed_asset = integration_dir.join(format!("claude-{}.json", context.run_id));
    std::fs::create_dir(&managed_asset).unwrap();
    let mut events = HostAgentEvents::open(&paths).unwrap();

    assert!(events
        .reconcile_run(context, &binding, AgentProcessObservation::Missing)
        .is_err());
    assert!(events.list_runs(None).unwrap().is_empty());
    assert_eq!(
        events
            .snapshot(context.run_id)
            .unwrap()
            .unwrap()
            .snapshot
            .state,
        AgentState::Exited
    );
    assert!(managed_asset.is_dir());
    assert!(events
        .reconcile_run(context, &binding, AgentProcessObservation::Live)
        .is_err());
}

#[test]
fn listed_run_wire_shape_contains_no_provider_or_terminal_payload_fields() {
    let (_root, paths, registry) = setup();
    let (context, _) = bound_run(&registry, &paths, Provider::Claude);
    let mut events = HostAgentEvents::open(&paths).unwrap();
    events.append(context, AgentEventKind::Working).unwrap();
    let encoded = serde_json::to_string(&events.list_runs(None).unwrap()).unwrap();
    for forbidden in [
        "prompt",
        "response",
        "command",
        "terminal_text",
        "tool_input",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "unexpected field: {forbidden}"
        );
    }
    assert!(encoded.contains("zellij_pane_id"));
    assert!(encoded.contains("snapshot"));
}

#[test]
fn protocol_rejects_worktrunk_force_fields() {
    let run = serde_json::json!({
        "request_id": 1,
        "protocol_version": PROTOCOL_VERSION,
        "method": "worktrunk_remove",
        "params": {
            "workspace_id": WorkspaceId::new(),
            "target_path": "/repo-feature",
            "approval": null,
            "force": true
        }
    });
    assert!(serde_json::from_value::<HelperRequest>(run).is_err());

    let legacy_path_only_removal = serde_json::json!({
        "request_id": 1,
        "protocol_version": PROTOCOL_VERSION,
        "method": "worktrunk_remove",
        "params": {
            "surviving_worktree": "/repo",
            "target": "/repo-feature",
            "approval": null
        }
    });
    assert!(serde_json::from_value::<HelperRequest>(legacy_path_only_removal).is_err());

    let legacy_boolean = serde_json::json!({
        "request_id": 1,
        "protocol_version": PROTOCOL_VERSION,
        "method": "worktrunk_switch",
        "params": {
            "repository_path": "/repo",
            "selector": "feature",
            "approved": true
        }
    });
    assert!(serde_json::from_value::<HelperRequest>(legacy_boolean).is_err());
}

#[test]
fn agent_run_protocol_rejects_payload_and_weak_reconciliation_fields() {
    let list_with_payload = serde_json::json!({
        "request_id": 1,
        "protocol_version": PROTOCOL_VERSION,
        "method": "list_agent_runs",
        "params": {"workspace_id": null, "terminal_text": "secret"}
    });
    assert!(serde_json::from_value::<HelperRequest>(list_with_payload).is_err());

    let weak_reconcile = serde_json::json!({
        "request_id": 2,
        "protocol_version": PROTOCOL_VERSION,
        "method": "reconcile_agent_run",
        "params": {
            "run_id": AgentRunId::new(),
            "observation": {"state": "missing"}
        }
    });
    assert!(serde_json::from_value::<HelperRequest>(weak_reconcile).is_err());
}

#[cfg(unix)]
#[test]
fn worktrunk_mutation_requires_explicit_approval() {
    use std::os::unix::fs::PermissionsExt;

    let (root, paths, registry) = setup();
    let binary = root.path().join("wt");
    std::fs::write(
        &binary,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'wt 0.72.0\\n'; exit 0; fi\nif [ \"$3\" = \"config\" ]; then printf '%s' '{\"state\":\"no_commands\",\"commands\":[],\"stale\":[]}'; exit 0; fi\nexit 99\n",
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let repository = root.path().join("repo");
    std::fs::create_dir(&repository).unwrap();
    let mut services = HostServices::with_worktrunk(paths, binary);
    let requests = [
        RequestOperation::Handshake {
            client_version: crate::BUILD_ID.to_owned(),
        },
        RequestOperation::WorktrunkSwitch {
            repository_path: repository.to_string_lossy().into_owned(),
            selector: "feature".to_owned(),
            approval: None,
        },
    ];
    let input = requests
        .into_iter()
        .enumerate()
        .map(|(index, operation)| {
            serde_json::to_string(&HelperRequest {
                request_id: index as u64 + 1,
                protocol_version: PROTOCOL_VERSION,
                operation,
            })
            .unwrap()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut output = Vec::new();
    serve_json_lines_with_extension(&registry, &mut services, Cursor::new(input), &mut output)
        .unwrap();
    let responses = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<HelperResponse>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(matches!(
        &responses[1].result,
        ResponseResult::Ok {
            payload: ResponsePayload::HostService {
                payload
            }
        } if matches!(
            payload.as_ref(),
            HostServicePayload::WorktrunkApprovalRequired { command, .. }
                if command.contains("switch") && !command.contains("--force")
        )
    ));
}

#[test]
fn provider_payload_secrets_never_cross_the_semantic_store_boundary() {
    let (_root, paths, registry) = setup();
    let host_id = registry.local_host_id().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let workspace = WorkspaceRecord::new(host_id, folder.path().to_string_lossy());
    registry.upsert_workspace(&workspace).unwrap();
    let arguments = ProviderHookArgs {
        provider: Provider::Codex,
        workspace_id: workspace.id,
        run_id: AgentRunId::new(),
        pane_id: Some(PaneId::new()),
    };
    let secret = "NEVER_PERSIST_THIS_PROVIDER_SECRET";
    let start = format!(r#"{{"hook_event_name":"SessionStart","prompt":"{secret}"}}"#);
    assert!(record_provider_hook(
        &paths,
        &registry,
        arguments,
        start.as_bytes()
    ));
    let events = HostAgentEvents::open(&paths).unwrap();
    let startup = events.follow(arguments.run_id, 0, 10).unwrap();
    assert_eq!(startup.len(), 2);
    assert_eq!(startup[0].update.event.sequence, 1);
    assert_eq!(startup[0].update.event.kind, healthy_event());
    assert_eq!(startup[1].update.event.sequence, 2);
    assert_eq!(startup[1].update.event.kind, AgentEventKind::Ready);
    assert_eq!(
        startup[1].update.snapshot.state,
        crate::agent_status::AgentState::Ready
    );
    drop(events);

    let working = format!(r#"{{"hook_event_name":"UserPromptSubmit","tool_input":"{secret}"}}"#);
    assert!(record_provider_hook(
        &paths,
        &registry,
        arguments,
        working.as_bytes()
    ));

    let mut events = HostAgentEvents::open(&paths).unwrap();
    let snapshot = events.snapshot(arguments.run_id).unwrap().unwrap();
    assert_eq!(
        snapshot.snapshot.state,
        crate::agent_status::AgentState::Working
    );
    assert_eq!(events.follow(arguments.run_id, 0, 10).unwrap().len(), 3);
    drop(events);

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            std::fs::metadata(paths.agent_events_path()).unwrap().mode() & 0o777,
            0o600
        );
    }

    for path in [
        paths.agent_events_path(),
        std::path::PathBuf::from(format!("{}-wal", paths.agent_events_path().display())),
        std::path::PathBuf::from(format!("{}-shm", paths.agent_events_path().display())),
    ] {
        if path.exists() {
            let bytes = std::fs::read(path).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains(secret));
        }
    }
}

#[test]
fn active_pane_cannot_be_silently_rebound_to_a_new_run() {
    let (_root, paths, registry) = setup();
    let host_id = registry.local_host_id().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let workspace = WorkspaceRecord::new(host_id, folder.path().to_string_lossy());
    registry.upsert_workspace(&workspace).unwrap();
    let pane_id = Some(PaneId::new());
    let first = AgentRunContext {
        host_id,
        workspace_id: workspace.id,
        run_id: AgentRunId::new(),
        pane_id,
        provider: Provider::Claude,
    };
    let second = AgentRunContext {
        run_id: AgentRunId::new(),
        ..first
    };
    let mut events = HostAgentEvents::open(&paths).unwrap();
    events.register_run(&registry, first).unwrap();
    assert!(events.register_run(&registry, second).is_err());

    assert!(events.register_run(&registry, first).is_ok());
    assert!(events.append(first, AgentEventKind::Working).is_ok());
    assert!(events.append(second, AgentEventKind::Working).is_err());
}
