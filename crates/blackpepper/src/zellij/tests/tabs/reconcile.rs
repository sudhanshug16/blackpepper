use super::*;

#[test]
fn ensure_tab_recovers_empty_success_after_the_named_pane_becomes_ready() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let clients =
        success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n9 terminal_1 zellij attach repo-main\n");
    let before = success(r#"[{"tab_id":0,"position":0,"name":"shell","active":true}]"#);
    let after = success(
        r#"[{"tab_id":0,"position":0,"name":"shell","active":false},{"tab_id":7,"position":1,"name":"service-api","active":true}]"#,
    );
    let mut host = ScriptedTransport::new([
        clients.clone(),
        before.clone(),
        success("\r\n"),
        before,
        after.clone(),
        success("[]"),
        after.clone(),
        ready_terminal_pane(7, "service-api"),
        clients,
        after,
        success(""),
    ]);

    let result = runtime
        .ensure_tab_with_reconcile_timeout(
            &mut host,
            "repo-main",
            "service-api",
            Path::new("/srv/repo"),
            Some(&HostCommand::new("api-server")),
            Duration::from_millis(200),
        )
        .unwrap();

    assert_eq!(result, (7, false));
    assert_eq!(
        wrapped_zellij_args(host.commands.last().unwrap(), "/opt/zellij"),
        ["--session", "repo-main", "action", "go-to-tab-by-id", "0"]
    );
    assert_eq!(
        host.commands
            .iter()
            .filter(
                |command| wrapped_zellij_args(command, "/opt/zellij").ends_with(&[
                    "action".to_string(),
                    "new-tab".to_string(),
                    "--layout-string".to_string(),
                    "layout { tab focus=false { pane; }; }".to_string(),
                    "--name".to_string(),
                    "service-api".to_string(),
                    "--cwd".to_string(),
                    "/srv/repo".to_string(),
                    "--".to_string(),
                    "api-server".to_string(),
                ])
            )
            .count(),
        1
    );
}

#[test]
fn ensure_tab_recovers_an_outer_command_timeout_without_retrying() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let clients = success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n");
    let before = success(r#"[{"tab_id":0,"position":0,"name":"shell","active":true}]"#);
    let after = success(
        r#"[{"tab_id":0,"position":0,"name":"shell","active":true},{"tab_id":7,"position":1,"name":"service-api","active":false}]"#,
    );
    let mut host = ScriptedTransport::with_results([
        Ok(clients),
        Ok(before),
        Err(TransportError::CommandTimedOut {
            process_id: 42,
            timeout_ms: 5_000,
            cancellation_error: None,
        }),
        Err(TransportError::CommandTimedOut {
            process_id: 43,
            timeout_ms: 250,
            cancellation_error: None,
        }),
        Ok(metadata_timeout("Timeout listing tabs")),
        Ok(after.clone()),
        Ok(success("\n")),
        Ok(after.clone()),
        Ok(metadata_timeout("Timeout listing panes")),
        Ok(after),
        Ok(ready_terminal_pane(7, "service-api")),
    ]);

    let result = runtime
        .ensure_tab_with_reconcile_timeout(
            &mut host,
            "repo-main",
            "service-api",
            Path::new("/srv/repo"),
            None,
            Duration::from_millis(250),
        )
        .unwrap();

    assert_eq!(result, (7, false));
    assert_eq!(
        host.commands
            .iter()
            .filter(|command| {
                wrapped_zellij_args(command, "/opt/zellij")
                    .windows(2)
                    .any(|args| args == ["new-tab", "--layout-string"])
            })
            .count(),
        1
    );
}

#[test]
fn ensure_tab_refuses_malformed_or_stale_creation_results() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let clients = success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n");
    let attached_clients =
        success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n9 terminal_1 zellij attach repo-main\n");
    let before = success(r#"[{"tab_id":0,"position":0,"name":"shell","active":true}]"#);

    let mut malformed = ScriptedTransport::new([
        attached_clients.clone(),
        before.clone(),
        success("warning\n"),
        attached_clients,
        success(
            r#"[{"tab_id":0,"position":0,"name":"shell","active":false},{"tab_id":7,"position":1,"name":"service-api","active":true}]"#,
        ),
        success(""),
    ]);
    let error = runtime
        .ensure_tab_with_reconcile_timeout(
            &mut malformed,
            "repo-main",
            "service-api",
            Path::new("/srv/repo"),
            None,
            Duration::ZERO,
        )
        .unwrap_err();
    assert!(error.to_string().contains("nonnumeric stdout"));
    assert_eq!(
        wrapped_zellij_args(malformed.commands.last().unwrap(), "/opt/zellij"),
        ["--session", "repo-main", "action", "go-to-tab-by-id", "0"]
    );

    let mut stderr_only = ScriptedTransport::new([
        success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n"),
        before.clone(),
        CommandOutput {
            success: true,
            status: Some(0),
            stdout: Vec::new(),
            stderr: b"unexpected warning\n".to_vec(),
        },
    ]);
    let error = runtime
        .ensure_tab_with_reconcile_timeout(
            &mut stderr_only,
            "repo-main",
            "service-api",
            Path::new("/srv/repo"),
            None,
            Duration::ZERO,
        )
        .unwrap_err();
    assert!(error.to_string().contains("19 stderr byte(s)"));
    assert_eq!(stderr_only.commands.len(), 3);

    let different_id = success(
        r#"[{"tab_id":0,"position":0,"name":"shell","active":true},{"tab_id":8,"position":1,"name":"service-api","active":false}]"#,
    );
    let mut stale = ScriptedTransport::new([clients, before, success("7\n"), different_id]);
    let error = runtime
        .ensure_tab_with_reconcile_timeout(
            &mut stale,
            "repo-main",
            "service-api",
            Path::new("/srv/repo"),
            None,
            Duration::ZERO,
        )
        .unwrap_err();
    assert!(error.to_string().contains("stale or mismatched"));
    assert_eq!(stale.commands.len(), 4);
}

fn metadata_timeout(message: &str) -> CommandOutput {
    CommandOutput {
        success: false,
        status: Some(2),
        stdout: Vec::new(),
        stderr: format!("{message}\n").into_bytes(),
    }
}

#[test]
fn ensure_tab_refuses_duplicate_names_before_or_after_creation() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let clients = success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n");
    let duplicate_before = success(
        r#"[{"tab_id":1,"position":0,"name":"service-api","active":true},{"tab_id":2,"position":1,"name":"service-api","active":false}]"#,
    );
    let mut before_host = ScriptedTransport::new([clients.clone(), duplicate_before]);
    let error = runtime
        .ensure_tab(
            &mut before_host,
            "repo-main",
            "service-api",
            Path::new("/srv/repo"),
            None,
        )
        .unwrap_err();
    assert!(error.to_string().contains("found 2 Zellij tabs"));
    assert_eq!(before_host.commands.len(), 2);

    let original = success(r#"[{"tab_id":0,"position":0,"name":"shell","active":true}]"#);
    let duplicate_after = success(
        r#"[{"tab_id":0,"position":0,"name":"shell","active":true},{"tab_id":7,"position":1,"name":"service-api","active":false},{"tab_id":8,"position":2,"name":"service-api","active":false}]"#,
    );
    let mut after_host = ScriptedTransport::new([clients, original, success(""), duplicate_after]);
    let error = runtime
        .ensure_tab_with_reconcile_timeout(
            &mut after_host,
            "repo-main",
            "service-api",
            Path::new("/srv/repo"),
            None,
            Duration::ZERO,
        )
        .unwrap_err();
    assert!(error.to_string().contains("produced 2 new Zellij tabs"));
    assert_eq!(after_host.commands.len(), 4);
}

#[test]
fn ensure_tab_times_out_unknown_creation_without_sending_a_second_mutation() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let clients = success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n");
    let before = success(r#"[{"tab_id":0,"position":0,"name":"shell","active":true}]"#);
    let mut host = ScriptedTransport::new([clients, before.clone(), success(""), before]);

    let error = runtime
        .ensure_tab_with_reconcile_timeout(
            &mut host,
            "repo-main",
            "service-api",
            Path::new("/srv/repo"),
            None,
            Duration::ZERO,
        )
        .unwrap_err();

    assert!(error.to_string().contains("no retry was sent"));
    assert_eq!(host.commands.len(), 4);
}
