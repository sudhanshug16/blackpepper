use super::*;
use std::fs;
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn codex_launch_preserves_permissions_and_uses_session_overrides() {
    let workspace = WorkspaceId::new();
    let run = AgentRunId::new();
    let pane = PaneId::new();
    let launch = build_launch(
        ProviderKind::Codex,
        workspace,
        run,
        pane,
        Path::new("/opt/blackpepper/bp-host"),
        Path::new("/tmp/blackpepper"),
    )
    .unwrap();
    assert_eq!(launch.args.len(), 12);
    assert!(launch
        .args
        .iter()
        .any(|arg| arg.contains("PermissionRequest")));
    assert!(!launch.args.join(" ").contains("bypass"));
    assert!(!launch.args.join(" ").contains(&workspace.to_string()));
    assert!(!launch.args.join(" ").contains(&run.to_string()));
    assert!(!launch.args.join(" ").contains(&pane.to_string()));
    assert!(launch.args.join(" ").contains("$BLACKPEPPER_PANE_ID"));
    assert_eq!(
        launch.env["BLACKPEPPER_WORKSPACE_ID"],
        workspace.to_string()
    );
    assert_eq!(launch.env["BLACKPEPPER_AGENT_RUN_ID"], run.to_string());
    assert_eq!(launch.env["BLACKPEPPER_PANE_ID"], pane.to_string());
    assert_eq!(
        launch.preflight_args().unwrap().last().map(String::as_str),
        Some("list")
    );
    assert!(launch.assets.is_empty());
}

#[test]
fn claude_uses_additional_settings_file() {
    let launch = build_launch(
        ProviderKind::Claude,
        WorkspaceId::new(),
        AgentRunId::new(),
        PaneId::new(),
        Path::new("/opt/bp-host"),
        Path::new("/state/integrations"),
    )
    .unwrap();
    assert_eq!(launch.args[0], "--settings");
    let settings: serde_json::Value = serde_json::from_slice(&launch.assets[0].contents).unwrap();
    assert!(settings["hooks"]["Stop"].is_array());
    assert_eq!(
        launch.preflight_args().unwrap().last().map(String::as_str),
        Some("doctor")
    );
}

#[test]
fn opencode_plugin_only_forwards_compact_fields() {
    let launch = build_launch(
        ProviderKind::OpenCode,
        WorkspaceId::new(),
        AgentRunId::new(),
        PaneId::new(),
        Path::new("/opt/bp-host"),
        Path::new("/state/integrations"),
    )
    .unwrap();
    let source = String::from_utf8(launch.assets[0].contents.clone()).unwrap();
    assert!(source.contains("session_id"));
    assert!(source.contains("--pane-id"));
    assert!(source.contains("blackpepper.integration.ready"));
    assert!(source.contains("blackpepper.integration.heartbeat"));
    assert!(source.contains("semantic_sequence"));
    assert!(source.contains("if (!SEMANTIC_TYPES.has(type)) return null"));
    assert!(source.contains("if (body !== null)"));
    assert!(source.contains(&format!(
        "const HEARTBEAT_MS = {OPENCODE_HEARTBEAT_INTERVAL_MS}"
    )));
    assert!(source.contains(".catch(() => undefined)"));
    assert!(source.contains("const body = compact(event)"));
    assert!(source.contains("JSON.stringify(body)"));
    for forbidden in [
        "prompt",
        "response",
        "command",
        "tool_input",
        "tool_output",
        "tool_response",
        "content",
        "terminal",
    ] {
        assert!(
            !source.contains(forbidden),
            "managed plugin forwarded forbidden field {forbidden}"
        );
    }
    assert!(launch.env["OPENCODE_CONFIG_CONTENT"].contains("plugin"));
    assert!(launch.preflight_args().is_none());
}

#[test]
fn managed_assets_are_private_and_atomic() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("managed").join("settings.json");
    let launch = ProviderLaunch {
        provider: ProviderKind::Claude,
        program: "claude".into(),
        args: Vec::new(),
        env: BTreeMap::new(),
        assets: vec![ManagedAsset {
            path: path.clone(),
            contents: b"{}".to_vec(),
        }],
        health_event: "session_start",
    };
    launch.install_assets().unwrap();
    launch.install_assets().unwrap();
    assert_eq!(fs::read(path).unwrap(), b"{}");
    #[cfg(unix)]
    {
        assert_eq!(
            fs::metadata(temp.path().join("managed"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(temp.path().join("managed/settings.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn handshake_errors_are_provider_specific_and_never_suggest_bypassing_trust() {
    for provider in [
        ProviderKind::Codex,
        ProviderKind::Claude,
        ProviderKind::OpenCode,
    ] {
        let launch = build_launch(
            provider,
            WorkspaceId::new(),
            AgentRunId::new(),
            PaneId::new(),
            Path::new("/opt/bp-host"),
            Path::new("/state/integrations"),
        )
        .unwrap();
        let error = launch.handshake_error(42);
        assert!(error.contains("42"));
        assert!(!error.contains("dangerously"));
    }
}
