use super::*;
use crate::core::{AgentRunId, HostId, WorkspaceId};

fn tracker(run_id: AgentRunId) -> AgentStatusTracker {
    AgentStatusTracker::new(
        run_id,
        Provider::Codex,
        NeedsInputCapability::ProviderEventsWithOverlay,
    )
}

fn draft(run_id: AgentRunId, kind: AgentEventKind) -> AgentEventDraft {
    let host_id = HostId::from_uuid(uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("host:{run_id}").as_bytes(),
    ));
    let workspace_id = WorkspaceId::from_uuid(uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("workspace:{run_id}").as_bytes(),
    ));
    AgentEventDraft {
        host_id,
        workspace_id,
        run_id,
        pane_id: None,
        provider: Provider::Codex,
        observed_at_ms: 100,
        source: AgentEventSource::ProviderIntegration,
        kind,
    }
}

fn store() -> (tempfile::TempDir, AgentEventStore) {
    let directory = tempfile::tempdir().unwrap();
    let store = AgentEventStore::open(directory.path().join("agent-events.sqlite")).unwrap();
    (directory, store)
}

#[test]
fn append_allocates_per_run_sequences_and_replays_snapshots() {
    let (_directory, mut store) = store();
    let run_id = AgentRunId::new();
    let mut tracker = tracker(run_id);

    let first = store
        .append(&mut tracker, draft(run_id, AgentEventKind::Working))
        .unwrap();
    let second = store
        .append(&mut tracker, draft(run_id, AgentEventKind::TurnCompleted))
        .unwrap();

    assert_eq!(first.event.sequence, 1);
    assert_eq!(second.event.sequence, 2);
    assert_eq!(second.snapshot.state, AgentState::Done);
    assert_eq!(
        store.snapshot(run_id).unwrap(),
        Some(second.snapshot.clone())
    );
    assert_eq!(
        store.follow_after(run_id, 0, 10).unwrap(),
        vec![first, second]
    );
    assert_eq!(store.follow_after(run_id, 1, 10).unwrap().len(), 1);
}

#[test]
fn sequence_allocation_is_independent_per_run() {
    let (_directory, mut store) = store();
    let first_run = AgentRunId::new();
    let second_run = AgentRunId::new();
    let mut first_tracker = tracker(first_run);
    let mut second_tracker = tracker(second_run);

    assert_eq!(
        store
            .append(
                &mut first_tracker,
                draft(first_run, AgentEventKind::Working)
            )
            .unwrap()
            .event
            .sequence,
        1
    );
    assert_eq!(
        store
            .append(
                &mut second_tracker,
                draft(second_run, AgentEventKind::Working),
            )
            .unwrap()
            .event
            .sequence,
        1
    );
}

#[test]
fn cross_run_and_stale_trackers_are_rejected_without_mutation() {
    let (_directory, mut store) = store();
    let run_id = AgentRunId::new();
    let other_run = AgentRunId::new();
    let mut current = tracker(run_id);
    let before = current.snapshot();

    assert!(matches!(
        store.append(&mut current, draft(other_run, AgentEventKind::Working)),
        Err(AgentEventStoreError::CrossRun { .. })
    ));
    assert_eq!(current.snapshot(), before);
    assert_eq!(store.snapshot(other_run).unwrap(), None);

    store
        .append(&mut current, draft(run_id, AgentEventKind::Working))
        .unwrap();
    let mut stale = current.clone();
    store
        .append(&mut current, draft(run_id, AgentEventKind::TurnCompleted))
        .unwrap();
    let stale_before = stale.snapshot();
    assert!(matches!(
        store.append(&mut stale, draft(run_id, AgentEventKind::Working)),
        Err(AgentEventStoreError::StaleTracker { .. })
    ));
    assert_eq!(stale.snapshot(), stale_before);
    assert_eq!(store.follow_after(run_id, 0, 10).unwrap().len(), 2);
}

#[test]
fn rejected_event_rolls_back_sequence_and_snapshot() {
    let (_directory, mut store) = store();
    let run_id = AgentRunId::new();
    let mut tracker = tracker(run_id);
    let mut invalid = draft(run_id, AgentEventKind::Working);
    invalid.source = AgentEventSource::ProcessSupervisor;

    assert!(matches!(
        store.append(&mut tracker, invalid),
        Err(AgentEventStoreError::TrackerRejected(
            IgnoredUpdate::InvalidSource
        ))
    ));
    assert_eq!(store.snapshot(run_id).unwrap(), None);

    let committed = store
        .append(&mut tracker, draft(run_id, AgentEventKind::Working))
        .unwrap();
    assert_eq!(committed.event.sequence, 1);
}

#[test]
fn rejected_batch_rolls_back_every_event() {
    let (_directory, mut store) = store();
    let run_id = AgentRunId::new();
    let mut tracker = tracker(run_id);
    let before = tracker.snapshot();
    let first = draft(
        run_id,
        AgentEventKind::IntegrationHealthChanged {
            health: IntegrationHealth::Healthy {
                integration_version: Some(1),
            },
        },
    );
    let mut invalid = draft(run_id, AgentEventKind::Ready);
    invalid.source = AgentEventSource::ProcessSupervisor;

    assert!(matches!(
        store.append_batch(&mut tracker, &[first, invalid]),
        Err(AgentEventStoreError::TrackerRejected(
            IgnoredUpdate::InvalidSource
        ))
    ));
    assert_eq!(tracker.snapshot(), before);
    assert_eq!(store.snapshot(run_id).unwrap(), None);
    assert!(store.follow_after(run_id, 0, 10).unwrap().is_empty());
}

#[test]
fn context_cannot_change_within_a_run() {
    let (_directory, mut store) = store();
    let run_id = AgentRunId::new();
    let mut tracker = tracker(run_id);
    store
        .append(&mut tracker, draft(run_id, AgentEventKind::Working))
        .unwrap();
    let mut changed = draft(run_id, AgentEventKind::TurnCompleted);
    changed.workspace_id = WorkspaceId::new();

    assert!(matches!(
        store.append(&mut tracker, changed),
        Err(AgentEventStoreError::ContextMismatch(id)) if id == run_id
    ));
    assert_eq!(store.follow_after(run_id, 0, 10).unwrap().len(), 1);
}

#[test]
fn store_uses_wal_and_persists_only_semantic_json() {
    let (directory, mut store) = store();
    assert_eq!(store.journal_mode().unwrap(), "wal");
    let run_id = AgentRunId::new();
    let mut tracker = tracker(run_id);
    store
        .append(&mut tracker, draft(run_id, AgentEventKind::NeedsInput))
        .unwrap();
    drop(store);

    let connection =
        rusqlite::Connection::open(directory.path().join("agent-events.sqlite")).unwrap();
    let mut statement = connection
        .prepare("PRAGMA table_info(agent_status_events)")
        .unwrap();
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        columns,
        vec!["run_id", "sequence", "event_json", "snapshot_json"]
    );
    assert!(columns
        .iter()
        .all(|name| !name.contains("payload") && !name.contains("viewport")));
}

#[test]
fn explain_reports_redacted_latest_authority() {
    let (_directory, mut store) = store();
    let run_id = AgentRunId::new();
    let mut tracker = tracker(run_id);
    store
        .append(&mut tracker, draft(run_id, AgentEventKind::Working))
        .unwrap();

    let explain = store.explain(run_id).unwrap().unwrap();
    assert_eq!(explain.authority, StatusAuthority::ProviderIntegration);
    assert_eq!(explain.last_event_kind, Some(AgentEventKind::Working));
    assert_eq!(explain.last_event_sequence, Some(1));

    let encoded = serde_json::to_string(&explain).unwrap();
    for forbidden in [
        "prompt",
        "response",
        "command",
        "tool_content",
        "terminal_text",
        "viewport",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "leaked field name {forbidden}"
        );
    }
}

#[test]
fn pre_cursor_freshness_schema_migrates_fail_closed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("agent-events.sqlite");
    let run_id = AgentRunId::new();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE agent_integration_freshness (
               run_id TEXT PRIMARY KEY NOT NULL,
               provider TEXT NOT NULL,
               last_seen_at_ms INTEGER NOT NULL,
               integration_version INTEGER
             );",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO agent_integration_freshness
               (run_id, provider, last_seen_at_ms, integration_version)
             VALUES (?1, 'opencode', 99, 1)",
            [run_id.to_string()],
        )
        .unwrap();
    drop(connection);

    let store = AgentEventStore::open(&path).unwrap();
    let migrated = store.integration_freshness(run_id).unwrap().unwrap();
    assert_eq!(migrated.semantic_sequence, 0);
    assert!(migrated.delivery_gap);
}

#[test]
fn draft_json_rejects_raw_payload_and_viewport_fields() {
    let run_id = AgentRunId::new();
    let encoded = format!(
        r#"{{"host_id":"{}","workspace_id":"{}","run_id":"{run_id}","pane_id":null,"provider":"codex","observed_at_ms":1,"source":"provider_integration","kind":{{"type":"working"}},"viewport":"secret"}}"#,
        HostId::new(),
        WorkspaceId::new(),
    );
    assert!(serde_json::from_str::<AgentEventDraft>(&encoded).is_err());

    let encoded = encoded.replace("viewport", "raw_payload");
    assert!(serde_json::from_str::<AgentEventDraft>(&encoded).is_err());

    let nested = format!(
        r#"{{"host_id":"{}","workspace_id":"{}","run_id":"{run_id}","pane_id":null,"provider":"codex","observed_at_ms":1,"source":"provider_integration","kind":{{"type":"working","raw_payload":"secret"}}}}"#,
        HostId::new(),
        WorkspaceId::new(),
    );
    assert!(serde_json::from_str::<AgentEventDraft>(&nested).is_err());
}
