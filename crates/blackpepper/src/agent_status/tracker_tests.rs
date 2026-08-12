use crate::core::{AgentRunId, HostId, WorkspaceId};

use super::*;

fn event(
    run_id: AgentRunId,
    sequence: u64,
    observed_at_ms: u64,
    kind: AgentEventKind,
) -> AgentEvent {
    let source = match kind {
        AgentEventKind::Exited { .. } => AgentEventSource::ProcessSupervisor,
        _ => AgentEventSource::ProviderIntegration,
    };
    AgentEvent {
        host_id: HostId::new(),
        workspace_id: WorkspaceId::new(),
        run_id,
        pane_id: None,
        provider: Provider::Codex,
        sequence,
        observed_at_ms,
        source,
        kind,
    }
}

fn supervisor_event(
    run_id: AgentRunId,
    sequence: u64,
    observed_at_ms: u64,
    kind: AgentEventKind,
) -> AgentEvent {
    let mut event = event(run_id, sequence, observed_at_ms, kind);
    event.source = AgentEventSource::ProcessSupervisor;
    event
}

fn snapshot(disposition: EventDisposition) -> AgentSnapshot {
    match disposition {
        EventDisposition::Applied(snapshot) => snapshot,
        EventDisposition::Ignored(reason) => panic!("event was ignored: {reason:?}"),
    }
}

fn blocker(run_id: AgentRunId, sequence: u64, observed_at_ms: u64) -> BlockerObservation {
    BlockerObservation {
        run_id,
        sequence,
        observed_at_ms,
        blocker: Some(BlockerExplain {
            provider: Provider::Codex,
            manifest_version: "1.0.0".to_string(),
            rule_id: "approval".to_string(),
            confidence: BlockerConfidence::High,
            priority: 100,
        }),
    }
}

#[test]
fn completion_is_done_until_this_client_marks_it_seen() {
    let run_id = AgentRunId::new();
    let mut tracker = AgentStatusTracker::new(
        run_id,
        Provider::Codex,
        NeedsInputCapability::ProviderEventsWithOverlay,
    );

    assert_eq!(
        snapshot(tracker.apply_event(event(run_id, 1, 10, AgentEventKind::Ready))).state,
        AgentState::Ready
    );
    assert_eq!(
        snapshot(tracker.apply_event(event(run_id, 2, 20, AgentEventKind::Working))).state,
        AgentState::Working
    );
    let completed =
        snapshot(tracker.apply_event(event(run_id, 3, 30, AgentEventKind::TurnCompleted)));
    assert_eq!(completed.state, AgentState::Done);
    assert!(completed.has_unseen_completion());

    let seen = tracker.mark_seen();
    assert_eq!(seen.state, AgentState::Ready);
    assert!(!seen.has_unseen_completion());

    tracker.apply_event(event(run_id, 4, 40, AgentEventKind::Working));
    let second = snapshot(tracker.apply_event(event(run_id, 5, 50, AgentEventKind::TurnCompleted)));
    assert_eq!(second.state, AgentState::Done);
    assert_eq!(second.completion_revision, 2);
}

#[test]
fn seen_state_is_local_to_each_tracker() {
    let run_id = AgentRunId::new();
    let mut first = AgentStatusTracker::new(
        run_id,
        Provider::Codex,
        NeedsInputCapability::ProviderEventsWithOverlay,
    );
    let mut second = AgentStatusTracker::new(
        run_id,
        Provider::Codex,
        NeedsInputCapability::ProviderEventsWithOverlay,
    );
    let completed = event(run_id, 1, 10, AgentEventKind::TurnCompleted);
    first.apply_event(completed.clone());
    second.apply_event(completed);

    first.mark_seen();
    assert_eq!(first.snapshot().state, AgentState::Ready);
    assert_eq!(second.snapshot().state, AgentState::Done);
}

#[test]
fn interrupted_run_suppresses_delayed_completion_across_rehydration() {
    let run_id = AgentRunId::new();
    let mut tracker = AgentStatusTracker::new(
        run_id,
        Provider::Codex,
        NeedsInputCapability::ProviderEventsWithOverlay,
    );
    tracker.apply_event(event(run_id, 1, 10, AgentEventKind::Working));
    let interrupted = snapshot(tracker.apply_event(supervisor_event(
        run_id,
        2,
        20,
        AgentEventKind::StateUnknown,
    )));
    assert_eq!(interrupted.state, AgentState::Unknown);
    assert!(interrupted.completion_suppressed);

    let mut rehydrated = AgentStatusTracker::from_snapshot(interrupted);
    let delayed =
        snapshot(rehydrated.apply_event(event(run_id, 3, 30, AgentEventKind::TurnCompleted)));
    assert_eq!(delayed.state, AgentState::Unknown);
    assert_eq!(delayed.completion_revision, 0);
    assert!(delayed.completion_suppressed);

    let resumed = snapshot(rehydrated.apply_event(event(run_id, 4, 40, AgentEventKind::Working)));
    assert_eq!(resumed.state, AgentState::Working);
    assert!(!resumed.completion_suppressed);
}

#[test]
fn visible_blocker_only_overlays_and_can_clear() {
    let run_id = AgentRunId::new();
    let mut tracker = AgentStatusTracker::new(
        run_id,
        Provider::Codex,
        NeedsInputCapability::BlockerOverlay,
    );
    tracker.apply_event(event(run_id, 1, 10, AgentEventKind::Working));

    let applied = tracker.apply_blocker(blocker(run_id, 1, 11));
    let BlockerDisposition::Applied { snapshot, changed } = applied else {
        panic!("blocker should apply");
    };
    assert!(changed);
    assert_eq!(snapshot.state, AgentState::NeedsInput);

    let cleared = tracker.apply_blocker(BlockerObservation {
        run_id,
        sequence: 2,
        observed_at_ms: 12,
        blocker: None,
    });
    let BlockerDisposition::Applied { snapshot, changed } = cleared else {
        panic!("clear should apply");
    };
    assert!(changed);
    assert_eq!(snapshot.state, AgentState::Working);
}

#[test]
fn overlay_never_revives_an_exited_run() {
    let run_id = AgentRunId::new();
    let mut tracker = AgentStatusTracker::new(
        run_id,
        Provider::Codex,
        NeedsInputCapability::BlockerOverlay,
    );
    tracker.apply_event(event(
        run_id,
        1,
        10,
        AgentEventKind::Exited { exit_code: Some(0) },
    ));
    tracker.apply_blocker(blocker(run_id, 1, 11));
    assert_eq!(tracker.snapshot().state, AgentState::Exited);
    assert_eq!(tracker.explain().blocker, None);
}

#[test]
fn old_run_and_out_of_order_updates_are_rejected() {
    let old_run = AgentRunId::new();
    let new_run = AgentRunId::new();
    let mut tracker = AgentStatusTracker::new(
        old_run,
        Provider::Codex,
        NeedsInputCapability::ProviderEventsWithOverlay,
    );
    tracker.apply_event(event(old_run, 9, 90, AgentEventKind::Working));
    tracker.begin_run(
        new_run,
        Provider::Codex,
        NeedsInputCapability::ProviderEventsWithOverlay,
    );

    assert_eq!(
        tracker.apply_event(event(old_run, 10, 100, AgentEventKind::TurnCompleted)),
        EventDisposition::Ignored(IgnoredUpdate::StaleRun)
    );
    tracker.apply_event(event(new_run, 4, 110, AgentEventKind::Working));
    assert_eq!(
        tracker.apply_event(event(new_run, 4, 120, AgentEventKind::TurnCompleted)),
        EventDisposition::Ignored(IgnoredUpdate::StaleSequence)
    );
    assert_eq!(
        tracker.apply_blocker(blocker(old_run, 1, 130)),
        BlockerDisposition::Ignored(IgnoredUpdate::StaleRun)
    );
}

#[test]
fn viewport_captured_before_newer_provider_event_is_rejected() {
    let run_id = AgentRunId::new();
    let mut tracker = AgentStatusTracker::new(
        run_id,
        Provider::Codex,
        NeedsInputCapability::ProviderEventsWithOverlay,
    );
    tracker.apply_event(event(run_id, 1, 100, AgentEventKind::Working));

    assert_eq!(
        tracker.apply_blocker(blocker(run_id, 1, 99)),
        BlockerDisposition::Ignored(IgnoredUpdate::StaleObservation)
    );
    assert_eq!(tracker.snapshot().state, AgentState::Working);
}

#[test]
fn unhealthy_integration_fails_closed_but_blocker_still_adds_attention() {
    let run_id = AgentRunId::new();
    let mut tracker = AgentStatusTracker::new(
        run_id,
        Provider::Codex,
        NeedsInputCapability::ProviderEventsWithOverlay,
    );
    tracker.apply_event(event(run_id, 1, 10, AgentEventKind::Working));
    let degraded = snapshot(tracker.apply_event(event(
        run_id,
        2,
        20,
        AgentEventKind::IntegrationHealthChanged {
            health: IntegrationHealth::Degraded {
                issue: IntegrationIssue::TransportUnavailable,
            },
        },
    )));
    assert_eq!(degraded.state, AgentState::Unknown);

    let BlockerDisposition::Applied { snapshot, .. } =
        tracker.apply_blocker(blocker(run_id, 1, 21))
    else {
        panic!("blocker should apply");
    };
    assert_eq!(snapshot.state, AgentState::NeedsInput);
}

#[test]
fn opencode_semantic_event_cannot_bypass_explicit_plugin_health() {
    let run_id = AgentRunId::new();
    let mut tracker = AgentStatusTracker::new(
        run_id,
        Provider::OpenCode,
        NeedsInputCapability::ProviderEvents,
    );
    let mut working = event(run_id, 1, 1, AgentEventKind::Working);
    working.provider = Provider::OpenCode;
    working.source = AgentEventSource::ProviderIntegration;

    let EventDisposition::Applied(snapshot) = tracker.apply_event(working) else {
        panic!("OpenCode semantic event was rejected")
    };
    assert_eq!(snapshot.integration_health, IntegrationHealth::Unknown);
}

#[test]
fn explain_is_redacted_by_construction() {
    let run_id = AgentRunId::new();
    let overlay = BlockerOverlay::bundled(Provider::Codex).unwrap();
    let secret = "SECRET_TOKEN_123 allow command? press enter to confirm or esc to cancel";
    let observation = BlockerObservation::evaluate(
        run_id,
        1,
        10,
        &overlay,
        BlockerInput {
            viewport: secret,
            terminal_title: None,
        },
    );
    let mut tracker = AgentStatusTracker::new(
        run_id,
        Provider::Codex,
        NeedsInputCapability::BlockerOverlay,
    );
    tracker.apply_blocker(observation);

    let encoded = serde_json::to_string(&tracker.explain()).unwrap();
    assert!(!encoded.contains("SECRET_TOKEN_123"));
    assert!(!encoded.contains("allow command?"));
    assert!(encoded.contains("confirm_or_cancel"));
}

#[test]
fn source_and_capability_guards_reject_impossible_events() {
    let run_id = AgentRunId::new();
    let mut tracker = AgentStatusTracker::new(
        run_id,
        Provider::Codex,
        NeedsInputCapability::BlockerOverlay,
    );
    assert_eq!(
        tracker.apply_event(event(run_id, 1, 10, AgentEventKind::NeedsInput)),
        EventDisposition::Ignored(IgnoredUpdate::CapabilityMismatch)
    );

    let mut invalid = event(run_id, 2, 20, AgentEventKind::Working);
    invalid.source = AgentEventSource::ProcessSupervisor;
    assert_eq!(
        tracker.apply_event(invalid),
        EventDisposition::Ignored(IgnoredUpdate::InvalidSource)
    );
}
