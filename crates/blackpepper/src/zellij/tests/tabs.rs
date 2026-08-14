use super::*;

mod reconcile;

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
    let created_tabs = success(
        r#"[{"tab_id":3,"position":0,"name":"shell","active":false},{"tab_id":7,"position":1,"name":"service-api","active":true}]"#,
    );
    let mut host = ScriptedTransport::new([
        clients.clone(),
        tabs,
        success("7\n"),
        created_tabs.clone(),
        ready_terminal_pane(7, "service-api"),
        clients,
        created_tabs,
        success(""),
    ]);

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
    assert_eq!(host.timeouts.len(), 8);
    assert_eq!(
        host.timeouts[..3],
        [
            Duration::from_secs(2),
            Duration::from_secs(5),
            Duration::from_secs(5),
        ]
    );
    assert!(host.timeouts[3..5]
        .iter()
        .all(|timeout| !timeout.is_zero() && *timeout <= Duration::from_secs(5)));
    assert_eq!(host.timeouts[5], Duration::from_secs(2));
    assert_eq!(host.timeouts[6], Duration::from_secs(5));
    assert_eq!(host.timeouts[7], Duration::from_secs(5));
}

#[test]
fn ensure_tab_does_not_overwrite_a_users_new_focus_choice() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let clients =
        success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n9 terminal_1 zellij attach repo-main\n");
    let before = success(r#"[{"tab_id":3,"position":0,"name":"shell","active":true}]"#);
    let created = success(
        r#"[{"tab_id":3,"position":0,"name":"shell","active":false},{"tab_id":7,"position":1,"name":"service-api","active":true}]"#,
    );
    let user_choice = success(
        r#"[{"tab_id":3,"position":0,"name":"shell","active":false},{"tab_id":7,"position":1,"name":"service-api","active":false},{"tab_id":9,"position":2,"name":"notes","active":true}]"#,
    );
    let mut host = ScriptedTransport::new([
        clients.clone(),
        before,
        success("7\n"),
        created,
        ready_terminal_pane(7, "service-api"),
        clients,
        user_choice,
    ]);

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
        ["--session", "repo-main", "action", "list-tabs", "--json"]
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
fn verified_close_refuses_a_reused_tab_identity() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let no_clients = success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n");
    let reused_pane = ready_terminal_pane_with_command(7, "service-api", "RUN=reused");
    let mut host = ScriptedTransport::new([no_clients, reused_pane]);

    let closed = runtime
        .close_tab_if_pane_matches(
            &mut host,
            "repo-main",
            7,
            "service-api",
            "terminal_4",
            "RUN=expected",
        )
        .unwrap();

    assert!(!closed);
    assert_eq!(host.commands.len(), 2);
}

#[test]
fn verified_close_rechecks_identity_immediately_before_mutation() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let no_clients = success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n");
    let owned_pane = ready_terminal_pane_with_command(7, "service-api", "RUN=expected");
    let mut host = ScriptedTransport::new([no_clients, owned_pane, success("")]);

    let closed = runtime
        .close_tab_if_pane_matches(
            &mut host,
            "repo-main",
            7,
            "service-api",
            "terminal_4",
            "RUN=expected",
        )
        .unwrap();

    assert!(closed);
    assert_eq!(
        wrapped_zellij_args(host.commands.last().unwrap(), "/opt/zellij"),
        ["--session", "repo-main", "action", "close-tab-by-id", "7"]
    );
}

fn ready_terminal_pane(tab_id: u64, tab_name: &str) -> CommandOutput {
    ready_terminal_pane_with_command(tab_id, tab_name, "api-server")
}

fn ready_terminal_pane_with_command(
    tab_id: u64,
    tab_name: &str,
    terminal_command: &str,
) -> CommandOutput {
    success(
        &serde_json::json!([{
            "id": 4,
            "is_plugin": false,
            "tab_id": tab_id,
            "tab_name": tab_name,
            "exited": false,
            "exit_status": null,
            "is_held": false,
            "terminal_command": terminal_command,
            "pane_command": "api-server"
        }])
        .to_string(),
    )
}
