use super::super::agent_events::{AgentRunContext, HostAgentEvents};
use super::*;
use crate::agent_status::Provider;
use crate::core::{HostRegistry, WorkspaceRecord};

#[cfg(unix)]
#[test]
fn host_watcher_emits_redacted_transitions_only() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
    paths.prepare().unwrap();
    let mut registry = HostRegistry::open(paths.registry_path()).unwrap();
    let host_id = registry.ensure_local_host("test").unwrap();
    let workspace_folder = root.path().join("workspace");
    std::fs::create_dir(&workspace_folder).unwrap();
    let workspace = WorkspaceRecord::new(host_id, workspace_folder.to_string_lossy());
    registry.upsert_workspace(&workspace).unwrap();
    let arguments = BlockerWatchArgs {
        workspace_id: workspace.id,
        run_id: AgentRunId::new(),
        pane_id: PaneId::new(),
        provider: Provider::Codex,
        session: "test-session".to_owned(),
        zellij_version: "0.44.1".to_owned(),
        zellij_pane_id: "terminal_7".to_owned(),
        after_sequence: 8,
    };
    let mut events = HostAgentEvents::open(&paths).unwrap();
    events
        .register_run(
            &registry,
            AgentRunContext {
                host_id,
                workspace_id: workspace.id,
                run_id: arguments.run_id,
                pane_id: Some(arguments.pane_id),
                provider: Provider::Codex,
            },
        )
        .unwrap();
    drop(events);

    let binary = root.path().join("zellij");
    let fixture = include_str!("../status_monitor/fixtures/codex_blocked.ndjson");
    std::fs::write(
        &binary,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'zellij {}\\n'; exit 0; fi\nif [ \"$4\" = \"list-clients\" ]; then if [ \"$ZELLIJ_SOCKET_DIR\" = \"/tmp/zellij-$(/usr/bin/id -u)\" ]; then printf 'CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\\n'; exit 0; fi; printf \"Session '%s' not found. The following sessions are active:\\n\" \"$2\" >&2; exit 1; fi\nprintf '%s' '{}'\n",
            arguments.zellij_version,
            fixture.replace('\'', "'\\''")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut output = Vec::new();

    watch_blockers_with_binary(&paths, &registry, &arguments, &mut output, &binary).unwrap();
    let output = String::from_utf8(output).unwrap();
    let transition: crate::status_monitor::BlockerTransition =
        serde_json::from_str(output.trim()).unwrap();
    assert_eq!(transition.sequence, 9);
    assert_eq!(transition.run_id, arguments.run_id);
    assert!(!output.contains("sensitive-command-text"));
    assert!(!output.contains("\"viewport\":"));
}

#[test]
fn watch_arguments_require_stable_ids_pane_and_safe_zellij_version() {
    let workspace_id = WorkspaceId::new();
    let run_id = AgentRunId::new();
    let pane_id = PaneId::new();
    let valid = [
        "--workspace-id".to_owned(),
        workspace_id.to_string(),
        "--run-id".to_owned(),
        run_id.to_string(),
        "--pane-id".to_owned(),
        pane_id.to_string(),
        "--provider".to_owned(),
        "codex".to_owned(),
        "--session".to_owned(),
        "session".to_owned(),
        "--zellij-version".to_owned(),
        "0.44.1".to_owned(),
        "--zellij-pane-id".to_owned(),
        "terminal_7".to_owned(),
    ];
    assert!(BlockerWatchArgs::parse(valid.clone()).is_some());
    let missing_version = valid
        .chunks_exact(2)
        .filter(|pair| pair[0] != "--zellij-version")
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    assert!(BlockerWatchArgs::parse(missing_version).is_none());
    let mut invalid_version = valid.to_vec();
    let value = invalid_version
        .iter_mut()
        .skip_while(|value| value.as_str() != "--zellij-version")
        .nth(1)
        .unwrap();
    *value = "0.44.1 unexpected".to_owned();
    assert!(BlockerWatchArgs::parse(invalid_version).is_none());
    let mut traversal_version = valid.to_vec();
    let value = traversal_version
        .iter_mut()
        .skip_while(|value| value.as_str() != "--zellij-version")
        .nth(1)
        .unwrap();
    *value = "../0.44.1".to_owned();
    assert!(BlockerWatchArgs::parse(traversal_version).is_none());
    let mut invalid = valid.to_vec();
    invalid.extend(["--viewport".to_owned(), "secret".to_owned()]);
    assert!(BlockerWatchArgs::parse(invalid).is_none());
}
