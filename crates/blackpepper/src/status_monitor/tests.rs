use std::io::Cursor;

use crate::agent_status::{
    AgentEvent, AgentEventKind, AgentEventSource, AgentState, AgentStatusTracker,
    BlockerConfidence, BlockerDisposition, IntegrationHealth, NeedsInputCapability, Provider,
};
use crate::core::{AgentRunId, HostId, PaneId, WorkspaceId};
use crate::zellij::ZellijRuntime;

use super::*;

const CODEX_BLOCKED: &str = include_str!("fixtures/codex_blocked.ndjson");
const CODEX_NEGATIVE: &str = include_str!("fixtures/codex_negative.ndjson");
const OPENCODE_BLOCKED: &str = include_str!("fixtures/opencode_blocked.ndjson");

pub(super) fn context(provider: Provider, integration_health: IntegrationHealth) -> MonitorContext {
    MonitorContext {
        host_id: HostId::new(),
        workspace_id: WorkspaceId::new(),
        run_id: AgentRunId::new(),
        pane_id: PaneId::new(),
        provider,
        integration_health,
    }
}

#[test]
fn zellij_subscription_is_json_and_has_no_scrollback_or_ansi() {
    let runtime = ZellijRuntime::new("/managed/zellij").unwrap();
    let command = runtime
        .subscribe_command("workspace-session", "terminal_7")
        .unwrap();
    assert_eq!(command.program, "/bin/sh");
    assert_eq!(
        &command.args[4..],
        [
            "--session",
            "workspace-session",
            "subscribe",
            "--pane-id",
            "terminal_7",
            "--format",
            "json"
        ]
    );
    assert_eq!(command.args[3], "/managed/zellij");
    assert!(!command.args.iter().any(|value| value == "--scrollback"));
    assert!(!command.args.iter().any(|value| value == "--ansi"));
    assert!(runtime
        .subscribe_command("workspace-session", "$(bad)")
        .is_err());
}

#[test]
fn captured_viewport_emits_needs_input_then_clear_only() {
    let context = context(
        Provider::Codex,
        IntegrationHealth::Healthy {
            integration_version: Some(1),
        },
    );
    let mut monitor = ViewportBlockerMonitor::bundled(context, "terminal_7").unwrap();
    let input = format!("{CODEX_BLOCKED}{CODEX_BLOCKED}{CODEX_NEGATIVE}");
    let mut now = 100;
    let mut transitions = Vec::new();
    let stats = consume_subscription(
        Cursor::new(input),
        &mut monitor,
        || {
            now += 1;
            now
        },
        |transition| transitions.push(transition),
    )
    .unwrap();

    assert_eq!(stats.lines, 3);
    assert_eq!(stats.transitions, 2);
    assert!(matches!(
        transitions[0].state,
        BlockerChange::NeedsInput {
            confidence: BlockerConfidence::High,
            ..
        }
    ));
    assert_eq!(transitions[0].sequence, 1);
    assert_eq!(transitions[1].state, BlockerChange::Cleared);
    assert_eq!(transitions[1].sequence, 2);
}

#[test]
fn restarted_monitor_can_resume_its_monotonic_sequence() {
    let mut monitor = ViewportBlockerMonitor::bundled_after(
        context(Provider::Codex, IntegrationHealth::Unknown),
        "terminal_7",
        41,
    )
    .unwrap();
    let transition = monitor
        .observe(
            "Allow command?\nPress enter to confirm or esc to cancel",
            None,
            1,
        )
        .unwrap();
    assert_eq!(transition.sequence, 42);
}

#[test]
fn negative_fixture_and_unrelated_pane_do_not_create_state() {
    let mut monitor = ViewportBlockerMonitor::bundled(
        context(Provider::Codex, IntegrationHealth::Unknown),
        "terminal_7",
    )
    .unwrap();
    let other = CODEX_BLOCKED.replace("terminal_7", "terminal_8");
    let input = format!("{CODEX_NEGATIVE}{other}");
    let mut transitions = Vec::new();
    let stats = consume_subscription(
        Cursor::new(input),
        &mut monitor,
        || 1,
        |transition| transitions.push(transition),
    )
    .unwrap();

    assert!(transitions.is_empty());
    assert_eq!(stats.ignored_other_panes, 1);
}

#[test]
fn pane_close_clears_overlay_without_claiming_exit() {
    let mut monitor = ViewportBlockerMonitor::bundled(
        context(Provider::Codex, IntegrationHealth::Unknown),
        "terminal_7",
    )
    .unwrap();
    let input = format!(
        "{CODEX_BLOCKED}{}",
        r#"{"event":"pane_closed","pane_id":"terminal_7"}
"#
    );
    let mut transitions = Vec::new();
    consume_subscription(
        Cursor::new(input),
        &mut monitor,
        || 1,
        |transition| transitions.push(transition),
    )
    .unwrap();
    assert_eq!(transitions.len(), 2);
    assert_eq!(transitions[1].state, BlockerChange::Cleared);
    assert!(!serde_json::to_string(&transitions[1])
        .unwrap()
        .contains("exited"));
}

#[test]
fn healthy_opencode_plugin_suppresses_screen_authority() {
    let context = context(
        Provider::OpenCode,
        IntegrationHealth::Healthy {
            integration_version: Some(1),
        },
    );
    let mut monitor = ViewportBlockerMonitor::bundled(context, "terminal_9").unwrap();
    let mut transitions = Vec::new();
    consume_subscription(
        Cursor::new(OPENCODE_BLOCKED),
        &mut monitor,
        || 10,
        |transition| transitions.push(transition),
    )
    .unwrap();
    assert!(transitions.is_empty());

    // The healthy watcher still caches only the redacted rule result. When a
    // heartbeat expires it can expose that match immediately, without a pane
    // repaint or retaining terminal text.
    let stale = monitor
        .set_integration_health(IntegrationHealth::Stale, 11)
        .unwrap();
    assert!(matches!(stale.state, BlockerChange::NeedsInput { .. }));
    consume_subscription(
        Cursor::new(OPENCODE_BLOCKED),
        &mut monitor,
        || 12,
        |transition| transitions.push(transition),
    )
    .unwrap();
    assert!(transitions.is_empty());
    let cleared = monitor
        .set_integration_health(
            IntegrationHealth::Healthy {
                integration_version: Some(1),
            },
            13,
        )
        .unwrap();
    assert_eq!(cleared.state, BlockerChange::Cleared);
}

#[test]
fn wire_transition_is_redacted_and_strict() {
    let mut monitor = ViewportBlockerMonitor::bundled(
        context(Provider::Codex, IntegrationHealth::Unknown),
        "terminal_7",
    )
    .unwrap();
    let mut transitions = Vec::new();
    consume_subscription(
        Cursor::new(CODEX_BLOCKED),
        &mut monitor,
        || 1,
        |transition| transitions.push(transition),
    )
    .unwrap();
    let json = serde_json::to_string(&transitions[0]).unwrap();
    assert!(!json.contains("sensitive-command-text"));
    assert!(!json.contains("Allow command"));
    assert!(!json.contains("\"viewport\":"));
    assert!(!json.contains("evidence"));

    let mut value = serde_json::to_value(&transitions[0]).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("viewport".into(), serde_json::json!(["secret"]));
    assert!(serde_json::from_value::<BlockerTransition>(value).is_err());

    let mut value = serde_json::to_value(&transitions[0]).unwrap();
    value["state"]["evidence"] = serde_json::json!("secret");
    assert!(serde_json::from_value::<BlockerTransition>(value).is_err());
}

#[test]
fn malformed_unknown_and_oversize_lines_are_skipped_without_echo() {
    let mut monitor = ViewportBlockerMonitor::bundled(
        context(Provider::Codex, IntegrationHealth::Unknown),
        "terminal_7",
    )
    .unwrap();
    let mut input =
        b"not-json-sensitive\n{\"event\":\"future_event\",\"secret\":\"hidden\"}\n".to_vec();
    input.extend(std::iter::repeat_n(b'x', MAX_SUBSCRIPTION_LINE_BYTES + 1));
    input.push(b'\n');
    input.extend_from_slice(CODEX_NEGATIVE.as_bytes());

    let stats = consume_subscription(Cursor::new(input), &mut monitor, || 1, |_| {}).unwrap();
    assert_eq!(stats.malformed, 1);
    assert_eq!(stats.unknown_events, 1);
    assert_eq!(stats.oversize, 1);
    assert_eq!(stats.transitions, 0);
}

#[test]
fn tracker_can_only_receive_overlay_and_restore_provider_state() {
    let context = context(Provider::Codex, IntegrationHealth::Unknown);
    let mut tracker = AgentStatusTracker::new(
        context.run_id,
        Provider::Codex,
        NeedsInputCapability::ProviderEventsWithOverlay,
    );
    tracker.apply_event(AgentEvent {
        host_id: context.host_id,
        workspace_id: context.workspace_id,
        run_id: context.run_id,
        pane_id: Some(context.pane_id),
        provider: Provider::Codex,
        sequence: 1,
        observed_at_ms: 1,
        source: AgentEventSource::ProviderIntegration,
        kind: AgentEventKind::Working,
    });
    let mut monitor = ViewportBlockerMonitor::bundled(context, "terminal_7").unwrap();
    let blocked = monitor
        .observe(
            "Allow command?\nPress enter to confirm or esc to cancel",
            None,
            2,
        )
        .unwrap();
    let BlockerDisposition::Applied { snapshot, .. } = tracker.apply_blocker(blocked.observation())
    else {
        panic!("screen overlay was rejected")
    };
    assert_eq!(snapshot.state, AgentState::NeedsInput);

    let cleared = monitor.observe("ordinary output", None, 3).unwrap();
    let BlockerDisposition::Applied { snapshot, .. } = tracker.apply_blocker(cleared.observation())
    else {
        panic!("screen clear was rejected")
    };
    assert_eq!(snapshot.state, AgentState::Working);
}
