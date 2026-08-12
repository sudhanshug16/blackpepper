use super::*;

#[test]
fn parser_rejects_unknown_duplicate_and_missing_flags() {
    let workspace = WorkspaceId::new();
    let run = AgentRunId::new();
    let valid = ProviderHookArgs::parse([
        "--provider".to_owned(),
        "codex".to_owned(),
        "--workspace-id".to_owned(),
        workspace.to_string(),
        "--run-id".to_owned(),
        run.to_string(),
    ]);
    assert!(valid.is_some());
    assert!(ProviderHookArgs::parse(["--provider".to_owned(), "codex".to_owned()]).is_none());
    assert!(ProviderHookArgs::parse([
        "--provider".to_owned(),
        "codex".to_owned(),
        "--provider".to_owned(),
        "claude".to_owned(),
        "--workspace-id".to_owned(),
        workspace.to_string(),
        "--run-id".to_owned(),
        run.to_string(),
    ])
    .is_none());
}

#[test]
fn reduction_keeps_only_semantic_state() {
    let value = serde_json::json!({
        "hook_event_name": "PermissionRequest",
        "prompt": "highly secret prompt",
        "tool_input": {"command": "rm -rf something"}
    });
    assert_eq!(
        reduce_provider_event(Provider::Codex, &value),
        Some(vec![AgentEventKind::NeedsInput])
    );
}

#[test]
fn opencode_uses_documented_tool_idle_and_permission_events() {
    for event in ["tool.execute.before", "tool.execute.after"] {
        assert_eq!(
            reduce_provider_event(Provider::OpenCode, &serde_json::json!({"type": event})),
            Some(vec![AgentEventKind::Working])
        );
    }
    assert_eq!(
        reduce_provider_event(
            Provider::OpenCode,
            &serde_json::json!({"type": "session.idle"})
        ),
        Some(vec![AgentEventKind::TurnCompleted])
    );
    assert_eq!(
        reduce_provider_event(
            Provider::OpenCode,
            &serde_json::json!({"type": "permission.asked"})
        ),
        Some(vec![AgentEventKind::NeedsInput])
    );
    assert_eq!(
        reduce_provider_event(
            Provider::OpenCode,
            &serde_json::json!({"type": "question.asked"})
        ),
        Some(vec![AgentEventKind::NeedsInput])
    );
    for event in [
        "permission.replied",
        "question.replied",
        "question.rejected",
    ] {
        assert_eq!(
            reduce_provider_event(Provider::OpenCode, &serde_json::json!({"type": event})),
            Some(vec![AgentEventKind::Working])
        );
    }
}

#[test]
fn opencode_health_requires_the_managed_plugin_handshake() {
    assert_eq!(
        reduce_provider_event(
            Provider::OpenCode,
            &serde_json::json!({"type": "blackpepper.integration.heartbeat"})
        ),
        Some(Vec::new())
    );
    assert_eq!(
        reduce_provider_event(
            Provider::OpenCode,
            &serde_json::json!({"type": "blackpepper.integration.ready"})
        ),
        Some(vec![healthy_event()])
    );
    assert_eq!(
        reduce_provider_event(
            Provider::OpenCode,
            &serde_json::json!({"type": "session.created"})
        ),
        Some(vec![AgentEventKind::Ready])
    );
    assert_eq!(
        reduce_provider_event(
            Provider::OpenCode,
            &serde_json::json!({"type": "server.connected"})
        ),
        None
    );
}

#[test]
fn session_start_orders_health_before_ready() {
    assert_eq!(
        reduce_provider_event(
            Provider::Codex,
            &serde_json::json!({"hook_event_name": "SessionStart"})
        ),
        Some(vec![healthy_event(), AgentEventKind::Ready])
    );
}

#[test]
fn oversized_hook_input_is_rejected() {
    let input = vec![b'x'; MAX_HOOK_INPUT_BYTES + 1];
    assert!(read_bounded(input.as_slice()).is_none());
}
