use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;

use portable_pty::PtySize;

use crate::transport::{
    CommandOutput, HostCommand, HostKind, HostTransport, LocalForward, PtyProcess, RunningCommand,
    TransportError,
};

use super::model::{
    classify_pane_process, client_list_reports_missing_session, parse_clients, parse_panes,
    parse_sessions, ClientOperation,
};
use super::runtime::{
    DEV_LAUNCHER_SCRIPT, LAUNCHER_ARG_ZERO, LAUNCHER_PROGRAM, PROD_LAUNCHER_SCRIPT,
};
use super::{PaneProcessState, ZellijError, ZellijRuntime};

#[test]
fn client_parser_requires_pinned_header() {
    let clients =
        parse_clients("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n1 terminal_2 codex --resume 7\n")
            .unwrap();
    assert_eq!(clients[0].client_id, 1);
    assert_eq!(clients[0].running_command, "codex --resume 7");
    assert!(parse_clients("1 terminal_2 codex").is_err());
}

#[test]
fn missing_session_attach_race_requires_the_exact_pre_pty_client_error() {
    let missing = CommandOutput {
        success: false,
        status: Some(1),
        stdout: Vec::new(),
        stderr: b"There is no active session!\n".to_vec(),
    };
    assert!(client_list_reports_missing_session(&missing, "repo-main"));

    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut ordinary_list = ScriptedTransport::new([missing.clone()]);
    assert!(matches!(
        runtime.list_clients(&mut ordinary_list, "repo-main"),
        Err(ZellijError::CommandFailed { .. })
    ));

    let mut host = ScriptedTransport::new([missing.clone()]);
    assert!(matches!(
        runtime.attach(
            &mut host,
            "repo-main",
            Path::new("/srv/repo"),
            PtySize::default(),
        ),
        Err(ZellijError::SessionMissingBeforeAttach)
    ));

    let mut wrong_status = missing.clone();
    wrong_status.status = Some(2);
    assert!(!client_list_reports_missing_session(
        &wrong_status,
        "repo-main"
    ));
    let mut unexpected_stdout = missing.clone();
    unexpected_stdout.stdout = b"partial output".to_vec();
    assert!(!client_list_reports_missing_session(
        &unexpected_stdout,
        "repo-main"
    ));
    let mut other_error = missing;
    other_error.stderr = b"There is no active session today!".to_vec();
    assert!(!client_list_reports_missing_session(
        &other_error,
        "repo-main"
    ));

    let detached_session_is_active = CommandOutput {
        success: true,
        status: Some(0),
        stdout: b"\x1b[32;1msome-other-session\x1b[m [Created \x1b[35;1m0s\x1b[m ago]\n".to_vec(),
        stderr: b"Session 'repo-main' not found. The following sessions are active:\n".to_vec(),
    };
    assert!(client_list_reports_missing_session(
        &detached_session_is_active,
        "repo-main"
    ));

    let attached_session_is_active = CommandOutput {
        success: false,
        status: Some(1),
        stdout: Vec::new(),
        stderr: b"Session 'repo-main' not found. The following sessions are active:\n\x1b[32;1msome-other-session\x1b[m\n".to_vec(),
    };
    assert!(client_list_reports_missing_session(
        &attached_session_is_active,
        "repo-main"
    ));

    let mut arbitrary_trailing_error = attached_session_is_active.clone();
    arbitrary_trailing_error.stderr =
        b"Session 'repo-main' not found. The following sessions are active:\ninvalid\0row\n"
            .to_vec();
    assert!(!client_list_reports_missing_session(
        &arbitrary_trailing_error,
        "repo-main"
    ));
}

#[test]
fn new_tab_uses_the_parseable_focus_false_layout() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let initial = HostCommand::new("codex").args(["--model", "gpt 5"]);
    let command = runtime
        .new_tab_command("repo-main", "agent", Path::new("/srv/repo"), Some(&initial))
        .unwrap();
    assert_eq!(
        wrapped_zellij_args(&command, "/opt/zellij"),
        [
            "--session",
            "repo-main",
            "action",
            "new-tab",
            "--layout-string",
            "layout { tab focus=false { pane; }; }",
            "--name",
            "agent",
            "--cwd",
            "/srv/repo",
            "--",
            "codex",
            "--model",
            "gpt 5"
        ]
    );
}

#[test]
fn ensure_tab_restores_the_only_attached_clients_previous_tab() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let clients =
        success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n9 terminal_1 zellij attach repo-main\n");
    let tabs = success(r#"[{"tab_id":3,"position":0,"name":"shell","active":true}]"#);
    let mut host =
        ScriptedTransport::new([clients.clone(), tabs, success("7\n"), clients, success("")]);

    let result = runtime
        .ensure_tab(
            &mut host,
            "repo-main",
            "service-api",
            Path::new("/srv/repo"),
            Some(&HostCommand::new("api-server")),
        )
        .unwrap();

    assert_eq!(result, (7, true));
    assert_eq!(
        wrapped_zellij_args(host.commands.last().unwrap(), "/opt/zellij"),
        ["--session", "repo-main", "action", "go-to-tab-by-id", "3"]
    );
    assert_eq!(
        host.timeouts,
        [
            Duration::from_secs(2),
            Duration::from_secs(5),
            Duration::from_secs(5),
            Duration::from_secs(2),
            Duration::from_secs(5),
        ]
    );
}

#[test]
fn ensure_tab_refuses_multiple_clients_before_creating_anything() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([success(
        "CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n9 terminal_1 zellij attach repo-main\n10 terminal_2 zellij attach repo-main\n",
    )]);

    let error = runtime
        .ensure_tab(
            &mut host,
            "repo-main",
            "service-api",
            Path::new("/srv/repo"),
            None,
        )
        .unwrap_err();

    assert!(error.to_string().contains("2 controlling client(s)"));
    assert_eq!(host.commands.len(), 1);
    assert_eq!(
        wrapped_zellij_args(&host.commands[0], "/opt/zellij"),
        ["--session", "repo-main", "action", "list-clients"]
    );
}

#[test]
fn initial_shell_focus_revalidates_one_exact_client_before_bounded_mutation() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let clients =
        success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n9 terminal_2 zellij attach repo-main\n");
    let tabs = success(
        r#"[{"tab_id":0,"position":0,"name":"shell","active":false},{"tab_id":1,"position":1,"name":"service","active":true}]"#,
    );
    let mut host = ScriptedTransport::new([clients.clone(), tabs, clients, success("")]);

    runtime
        .focus_initial_shell_for_single_client(&mut host, "repo-main")
        .unwrap();

    assert_eq!(
        wrapped_zellij_args(host.commands.last().unwrap(), "/opt/zellij"),
        ["--session", "repo-main", "action", "go-to-tab-by-id", "0"]
    );
    assert_eq!(
        host.timeouts,
        [
            Duration::from_secs(2),
            Duration::from_secs(5),
            Duration::from_secs(2),
            Duration::from_secs(5),
        ]
    );
}

#[test]
fn initial_shell_focus_refuses_zero_multiple_and_changed_clients_without_mutation() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut zero = ScriptedTransport::new([success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n")]);
    assert!(runtime
        .focus_initial_shell_for_single_client_with_timeout(&mut zero, "repo-main", Duration::ZERO,)
        .unwrap_err()
        .to_string()
        .contains("timed out"));
    assert_eq!(zero.commands.len(), 1);

    let mut multiple = ScriptedTransport::new([success(
        "CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n9 terminal_1 zellij attach repo-main\n10 terminal_2 zellij attach repo-main\n",
    )]);
    assert!(runtime
        .focus_initial_shell_for_single_client(&mut multiple, "repo-main")
        .unwrap_err()
        .to_string()
        .contains("found 2"));
    assert_eq!(multiple.commands.len(), 1);

    let before =
        success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n9 terminal_2 zellij attach repo-main\n");
    let tabs = success(r#"[{"tab_id":0,"position":0,"name":"shell","active":false}]"#);
    let changed = success(
        "CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n10 terminal_2 zellij attach repo-main\n",
    );
    let mut raced = ScriptedTransport::new([before, tabs, changed]);
    assert!(runtime
        .focus_initial_shell_for_single_client(&mut raced, "repo-main")
        .unwrap_err()
        .to_string()
        .contains("client set changed"));
    assert_eq!(raced.commands.len(), 3);
}

#[test]
fn initial_shell_focus_waits_for_the_reader_backed_client_to_appear() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let absent = success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n");
    let clients =
        success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n9 terminal_2 zellij attach repo-main\n");
    let tabs = success(
        r#"[{"tab_id":0,"position":0,"name":"shell","active":false},{"tab_id":1,"position":1,"name":"service","active":true}]"#,
    );
    let mut host = ScriptedTransport::new([absent, clients.clone(), tabs, clients, success("")]);

    runtime
        .focus_initial_shell_for_single_client_with_timeout(
            &mut host,
            "repo-main",
            Duration::from_millis(100),
        )
        .unwrap();

    assert_eq!(
        wrapped_zellij_args(host.commands.last().unwrap(), "/opt/zellij"),
        ["--session", "repo-main", "action", "go-to-tab-by-id", "0"]
    );
}

#[test]
fn new_session_shell_receives_workspace_environment() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let environment = std::collections::BTreeMap::from([
        ("API_HOST".to_string(), "127.0.0.1".to_string()),
        ("RUST_LOG".to_string(), "info".to_string()),
    ]);
    let command = runtime
        .create_session_with_env_command("repo-main", Path::new("/srv/repo"), &environment)
        .unwrap();

    assert_eq!(command.env, environment);
    assert_eq!(command.cwd.as_deref(), Some(Path::new("/srv/repo")));
    assert_eq!(
        wrapped_zellij_args(&command, "/opt/zellij"),
        ["attach", "--create-background", "--forget", "repo-main"]
    );
}

#[test]
fn missing_session_beside_an_attached_session_is_created() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let missing = CommandOutput {
        success: false,
        status: Some(1),
        stdout: Vec::new(),
        stderr: b"Session 'repo-main' not found. The following sessions are active:\n\x1b[32;1msome-other-session\x1b[m\n".to_vec(),
    };
    let mut host = ScriptedTransport::new([missing, success("")]);

    assert!(runtime
        .ensure_session(&mut host, "repo-main", Path::new("/srv/repo"))
        .unwrap());
    assert_eq!(host.commands.len(), 2);
    assert_eq!(
        wrapped_zellij_args(&host.commands[1], "/opt/zellij"),
        ["attach", "--create-background", "--forget", "repo-main"]
    );
}

#[test]
fn attached_client_forces_signals_to_detach_without_mutating_user_config() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let command = runtime
        .attach_command("repo-main", Path::new("/srv/repo"))
        .unwrap();

    assert_eq!(
        wrapped_zellij_args(&command, "/opt/zellij"),
        [
            "attach",
            "repo-main",
            "options",
            "--on-force-close",
            "detach"
        ]
    );
    assert_eq!(command.cwd.as_deref(), Some(Path::new("/srv/repo")));
    assert!(command.env.is_empty());
}

#[test]
fn attach_returns_the_pre_spawn_count_without_a_post_spawn_query() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([success(
        "CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n9 terminal_1 zellij attach repo-main\n",
    )]);

    let (mut process, clients) = runtime
        .attach(
            &mut host,
            "repo-main",
            Path::new("/srv/repo"),
            PtySize::default(),
        )
        .unwrap();

    assert_eq!(clients.len(), 1);
    assert_eq!(host.commands.len(), 1);
    assert_eq!(
        wrapped_zellij_args(&host.commands[0], "/opt/zellij"),
        ["--session", "repo-main", "action", "list-clients"]
    );
    process.kill().unwrap();
    let _ = process.wait().unwrap();
}

#[test]
fn client_observation_has_an_explicit_short_deadline() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n")]);

    runtime.list_clients(&mut host, "repo-main").unwrap();

    assert_eq!(host.timeouts, [Duration::from_secs(2)]);
}

#[test]
fn existing_session_is_not_mutated_with_new_environment() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n")]);
    let environment = std::collections::BTreeMap::from([(
        "CHANGED_AFTER_START".to_string(),
        "ignored".to_string(),
    )]);

    assert!(!runtime
        .ensure_session_with_env(&mut host, "repo-main", Path::new("/srv/repo"), &environment,)
        .unwrap());
    assert!(host.outputs.is_empty());
}

#[test]
fn runtime_checks_the_version_selected_for_a_retained_session() {
    let runtime = ZellijRuntime::for_version("/opt/zellij-0.44.1", "0.44.1").unwrap();
    let mut matching = ScriptedTransport::new([success("zellij 0.44.1\n")]);
    runtime.check_version(&mut matching).unwrap();

    let mut mismatched = ScriptedTransport::new([success("zellij 0.44.3\n")]);
    let error = runtime.check_version(&mut mismatched).unwrap_err();
    assert!(error.to_string().contains("0.44.1 is required"));
    assert!(error.to_string().contains("found 0.44.3"));
}

#[test]
fn default_runtime_still_requires_the_release_pin() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let version = format!("zellij {}\n", crate::transport::ZELLIJ_VERSION);
    let mut host = ScriptedTransport::new([success(&version)]);
    runtime.check_version(&mut host).unwrap();
    assert!(ZellijRuntime::for_version("/opt/zellij", "bad version").is_err());
    assert!(ZellijRuntime::for_version("/opt/zellij", "../0.44.3").is_err());
}

#[test]
fn configuration_check_is_read_only_and_uses_the_selected_binary() {
    let runtime = ZellijRuntime::for_version("/opt/zellij-0.44.1", "0.44.1").unwrap();
    let command = runtime.check_configuration_command();

    assert_eq!(
        wrapped_zellij_args(&command, "/opt/zellij-0.44.1"),
        ["setup", "--check"]
    );
    assert!(command.cwd.is_none());
    assert!(command.env.is_empty());

    let mut invalid = ScriptedTransport::new([CommandOutput {
        success: false,
        status: Some(1),
        stdout: Vec::new(),
        stderr: b"invalid config.kdl".to_vec(),
    }]);
    let error = runtime.check_configuration(&mut invalid).unwrap_err();
    assert!(error.to_string().contains("invalid config.kdl"));
}

#[test]
fn managed_configuration_is_an_explicit_single_file() {
    let runtime = ZellijRuntime::new("/opt/zellij")
        .unwrap()
        .with_config_file("/var/lib/blackpepper/zellij/config.kdl")
        .unwrap();
    let command = runtime.check_configuration_command();

    assert_eq!(
        wrapped_zellij_args(&command, "/opt/zellij"),
        [
            "--config",
            "/var/lib/blackpepper/zellij/config.kdl",
            "setup",
            "--check"
        ]
    );
    assert!(ZellijRuntime::new("/opt/zellij")
        .unwrap()
        .with_config_file("relative/config.kdl")
        .is_err());
}

#[test]
fn native_configuration_detection_distinguishes_absence_from_user_intent() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut absent = ScriptedTransport::new([success(
        "[CONFIG DIR]: Not Found\n[CONFIG FILE]: Not Found\n",
    )]);
    assert!(!runtime.user_configuration_present(&mut absent).unwrap());

    let mut configured = ScriptedTransport::new([success(
        "[LOOKING FOR CONFIG FILE FROM]: /home/me/.config/zellij/config.kdl\n[CONFIG FILE]: Well defined.\n",
    )]);
    assert!(runtime.user_configuration_present(&mut configured).unwrap());

    // An explicit config directory is user intent even when its config.kdl is
    // missing and pinned Zellij falls back to its built-in defaults.
    let mut explicit_directory = ScriptedTransport::new([success(
        "[LOOKING FOR CONFIG FILE FROM]: /owned/zellij/config.kdl\n[CONFIG ERROR]: missing\n",
    )]);
    assert!(runtime
        .user_configuration_present(&mut explicit_directory)
        .unwrap());

    let mut unfamiliar = ScriptedTransport::new([success("configuration okay\n")]);
    let error = runtime
        .user_configuration_present(&mut unfamiliar)
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("unfamiliar configuration diagnostics"));
}

#[test]
fn concurrent_clients_allow_attach_but_refuse_focus_affecting_mutations() {
    assert!(ClientOperation::Attach.allows(2));
    assert!(!ClientOperation::BackgroundMutation.allows(2));
    assert!(!ClientOperation::FocusChange.allows(2));
    assert!(!ClientOperation::Destroy.allows(2));
    assert!(ClientOperation::FocusChange.allows(1));
    assert!(ClientOperation::Destroy.allows(0));
}

#[test]
fn list_sessions_only_treats_the_pinned_no_sessions_exit_as_empty() {
    let none = CommandOutput {
        success: false,
        status: Some(1),
        stdout: Vec::new(),
        stderr: b"No active zellij sessions found.\n".to_vec(),
    };
    assert!(parse_sessions(none).unwrap().is_empty());

    let broken = CommandOutput {
        success: false,
        status: Some(1),
        stdout: Vec::new(),
        stderr: b"failed to connect to socket\n".to_vec(),
    };
    assert!(parse_sessions(broken).is_err());
}

#[test]
fn pane_parser_preserves_live_command_identity() {
    let panes = parse_panes(
        br#"[{
            "id": 7,
            "is_plugin": false,
            "tab_id": 3,
            "tab_name": "agent-run-1",
            "exited": false,
            "exit_status": null,
            "is_held": false,
            "terminal_command": "env BLACKPEPPER_AGENT_RUN_ID=run-1 codex",
            "pane_command": "codex"
        }]"#,
    )
    .unwrap();

    assert_eq!(panes[0].selector(), "terminal_7");
    assert_eq!(panes[0].process_state(), PaneProcessState::Live);
    assert_eq!(
        panes[0].terminal_command.as_deref(),
        Some("env BLACKPEPPER_AGENT_RUN_ID=run-1 codex")
    );
    assert_eq!(panes[0].pane_command.as_deref(), Some("codex"));
    assert_eq!(
        classify_pane_process(
            &panes,
            3,
            "agent-run-1",
            "terminal_7",
            "BLACKPEPPER_AGENT_RUN_ID=run-1",
        ),
        PaneProcessState::Live
    );
    assert_eq!(
        classify_pane_process(
            &panes,
            3,
            "replacement-tab",
            "terminal_7",
            "BLACKPEPPER_AGENT_RUN_ID=run-1",
        ),
        PaneProcessState::Live
    );
    assert_eq!(
        classify_pane_process(
            &panes,
            4,
            "agent-run-1",
            "terminal_7",
            "BLACKPEPPER_AGENT_RUN_ID=run-1",
        ),
        PaneProcessState::Live
    );
    assert_eq!(
        classify_pane_process(
            &panes,
            3,
            "agent-run-1",
            "terminal_8",
            "BLACKPEPPER_AGENT_RUN_ID=run-1",
        ),
        PaneProcessState::Missing
    );

    assert_eq!(
        classify_pane_process(
            &panes,
            3,
            "agent-run-1",
            "terminal_7",
            "BLACKPEPPER_AGENT_RUN_ID=different-run",
        ),
        PaneProcessState::UnverifiedIdentity {
            location_changed: false
        }
    );
}

#[test]
fn exited_and_held_panes_are_never_reported_live() {
    let panes = parse_panes(
        br#"[
          {
            "id": 4,
            "is_plugin": false,
            "tab_id": 1,
            "tab_name": "agent-run-2",
            "exited": true,
            "exit_status": 130,
            "is_held": false,
            "terminal_command": "claude",
            "pane_command": "claude"
          },
          {
            "id": 5,
            "is_plugin": false,
            "tab_id": 1,
            "tab_name": "agent-run-3",
            "exited": false,
            "exit_status": null,
            "is_held": true,
            "terminal_command": "opencode",
            "pane_command": "opencode"
          }
        ]"#,
    )
    .unwrap();

    assert_eq!(
        panes[0].process_state(),
        PaneProcessState::Exited { code: Some(130) }
    );
    assert_eq!(
        panes[1].process_state(),
        PaneProcessState::Exited { code: None }
    );
}

#[test]
fn pane_parser_fails_closed_when_lifecycle_fields_are_missing() {
    let missing_exited = br#"[{
        "id": 7,
        "is_plugin": false,
        "tab_id": 3,
        "tab_name": "agent-run-1",
        "exit_status": null,
        "is_held": false,
        "terminal_command": "codex",
        "pane_command": "codex"
    }]"#;
    assert!(parse_panes(missing_exited).is_err());

    let missing_status = br#"[{
        "id": 7,
        "is_plugin": false,
        "tab_id": 3,
        "tab_name": "agent-run-1",
        "exited": false,
        "is_held": false,
        "terminal_command": "codex",
        "pane_command": "codex"
    }]"#;
    assert!(parse_panes(missing_status).is_err());
}

#[test]
fn pane_parser_accepts_unavailable_best_effort_process_metadata() {
    let panes = parse_panes(
        br#"[{
            "id": 7,
            "is_plugin": false,
            "tab_id": 3,
            "tab_name": "agent-run-1",
            "exited": false,
            "exit_status": null,
            "is_held": false,
            "terminal_command": "codex"
        }]"#,
    )
    .unwrap();

    assert_eq!(panes[0].pane_command, None);
    assert_eq!(panes[0].process_state(), PaneProcessState::Live);
}

#[test]
fn pane_observation_distinguishes_exit_from_missing_identity() {
    let mut host = ScriptedTransport::new([
        success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n"),
        success(
            r#"[{
                "id": 7,
                "is_plugin": false,
                "tab_id": 3,
                "tab_name": "agent-run-1",
                "exited": true,
                "exit_status": 2,
                "is_held": true,
                "terminal_command": "env BLACKPEPPER_AGENT_RUN_ID=run-1 codex",
                "pane_command": "codex"
            }]"#,
        ),
    ]);
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();

    assert_eq!(
        runtime
            .pane_process_state(
                &mut host,
                "bp-session",
                3,
                "agent-run-1",
                "terminal_7",
                "BLACKPEPPER_AGENT_RUN_ID=run-1",
            )
            .unwrap(),
        PaneProcessState::Exited { code: Some(2) }
    );

    let mut reused = ScriptedTransport::new([
        success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n"),
        success(
            r#"[{
                "id": 7,
                "is_plugin": false,
                "tab_id": 3,
                "tab_name": "different-run",
                "exited": false,
                "exit_status": null,
                "is_held": false,
                "terminal_command": "env BLACKPEPPER_AGENT_RUN_ID=other-run codex",
                "pane_command": "codex"
            }]"#,
        ),
    ]);
    assert_eq!(
        runtime
            .pane_process_state(
                &mut reused,
                "bp-session",
                3,
                "agent-run-1",
                "terminal_7",
                "BLACKPEPPER_AGENT_RUN_ID=run-1",
            )
            .unwrap(),
        PaneProcessState::UnverifiedIdentity {
            location_changed: true
        }
    );
}

#[test]
fn pane_observation_reports_a_missing_session_without_querying_panes() {
    let mut host = ScriptedTransport::new([missing_session("bp-session")]);
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();

    assert_eq!(
        runtime
            .pane_process_state(
                &mut host,
                "bp-session",
                3,
                "agent-run-1",
                "terminal_7",
                "BLACKPEPPER_AGENT_RUN_ID=run-1",
            )
            .unwrap(),
        PaneProcessState::Missing
    );
    assert!(host.outputs.is_empty());
}

fn success(stdout: &str) -> CommandOutput {
    CommandOutput {
        success: true,
        status: Some(0),
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
    }
}

fn missing_session(session: &str) -> CommandOutput {
    CommandOutput {
        success: false,
        status: Some(1),
        stdout: Vec::new(),
        stderr: format!(
            "Session '{session}' not found. The following sessions are active:\nsome-other-session\n"
        )
        .into_bytes(),
    }
}

fn wrapped_zellij_args<'a>(command: &'a HostCommand, binary: &str) -> &'a [String] {
    assert_eq!(command.program, LAUNCHER_PROGRAM);
    let expected_script = if crate::IS_DEVELOPMENT_BUILD {
        DEV_LAUNCHER_SCRIPT
    } else {
        PROD_LAUNCHER_SCRIPT
    };
    assert_eq!(
        &command.args[..4],
        ["-c", expected_script, LAUNCHER_ARG_ZERO, binary]
    );
    &command.args[4..]
}

struct ScriptedTransport {
    outputs: VecDeque<CommandOutput>,
    commands: Vec<HostCommand>,
    timeouts: Vec<Duration>,
}

impl ScriptedTransport {
    fn new(outputs: impl IntoIterator<Item = CommandOutput>) -> Self {
        Self {
            outputs: outputs.into_iter().collect(),
            commands: Vec::new(),
            timeouts: Vec::new(),
        }
    }
}

impl HostTransport for ScriptedTransport {
    fn kind(&self) -> HostKind {
        HostKind::Local
    }

    fn spawn_exec(&mut self, _command: &HostCommand) -> Result<RunningCommand, TransportError> {
        Err(TransportError::Unsupported("not used by this test"))
    }

    fn spawn_exec_with_stdin(
        &mut self,
        _command: &HostCommand,
    ) -> Result<RunningCommand, TransportError> {
        Err(TransportError::Unsupported("not used by this test"))
    }

    fn attach_pty(
        &mut self,
        _command: &HostCommand,
        size: PtySize,
    ) -> Result<PtyProcess, TransportError> {
        PtyProcess::spawn(
            &crate::transport::ProcessSpec::new("sh").args(["-c", "exec sleep 30"]),
            size,
        )
    }

    fn forward_local_port(
        &mut self,
        _forward: LocalForward,
    ) -> Result<LocalForward, TransportError> {
        Err(TransportError::Unsupported("not used by this test"))
    }

    fn cancel_local_forward(&mut self, _forward: &LocalForward) -> Result<(), TransportError> {
        Err(TransportError::Unsupported("not used by this test"))
    }

    fn exec(&mut self, command: &HostCommand) -> Result<CommandOutput, TransportError> {
        self.commands.push(command.clone());
        self.outputs
            .pop_front()
            .ok_or(TransportError::Unsupported("unexpected command"))
    }

    fn exec_timeout(
        &mut self,
        command: &HostCommand,
        timeout: Duration,
    ) -> Result<CommandOutput, TransportError> {
        self.timeouts.push(timeout);
        self.exec(command)
    }
}
