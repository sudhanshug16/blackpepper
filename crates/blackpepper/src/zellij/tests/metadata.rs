use super::*;

#[test]
fn tab_metadata_retries_blank_success_before_parsing() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let tabs = r#"[{"tab_id":4,"position":0,"name":"shell","active":true}]"#;
    let mut host = ScriptedTransport::new([success("\r\n"), success(tabs)]);

    let listed = runtime.list_tabs(&mut host, "repo-main").unwrap();

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].tab_id, 4);
    assert_eq!(host.commands.len(), 2);
    assert!(host
        .timeouts
        .iter()
        .all(|timeout| !timeout.is_zero() && *timeout <= Duration::from_secs(2)));
}

#[test]
fn pane_metadata_retries_exact_zellij_timeout_before_parsing() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let pane = serde_json::json!([{
        "id": 9,
        "is_plugin": false,
        "tab_id": 4,
        "tab_name": "agent",
        "exited": false,
        "exit_status": null,
        "is_held": false,
        "terminal_command": "codex",
        "pane_command": "codex"
    }])
    .to_string();
    let mut host =
        ScriptedTransport::new([metadata_timeout("Timeout listing panes"), success(&pane)]);

    let listed = runtime.list_panes(&mut host, "repo-main").unwrap();

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].selector(), "terminal_9");
    assert_eq!(host.commands.len(), 2);
}

#[test]
fn tab_and_pane_metadata_retry_a_false_missing_session() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let tabs = r#"[{"tab_id":4,"position":0,"name":"shell","active":true}]"#;
    let pane = serde_json::json!([{
        "id": 9,
        "is_plugin": false,
        "tab_id": 4,
        "tab_name": "shell",
        "exited": false,
        "exit_status": null,
        "is_held": false,
        "terminal_command": "shell",
        "pane_command": "shell"
    }])
    .to_string();
    let mut tab_host = ScriptedTransport::new([no_active_session(), success(tabs)]);
    let mut pane_host = ScriptedTransport::new([no_active_session(), success(&pane)]);

    assert_eq!(
        runtime.list_tabs(&mut tab_host, "repo-main").unwrap()[0].tab_id,
        4
    );
    assert_eq!(
        runtime.list_panes(&mut pane_host, "repo-main").unwrap()[0].selector(),
        "terminal_9"
    );
    assert_eq!(tab_host.commands.len(), 2);
    assert_eq!(pane_host.commands.len(), 2);
}

#[test]
fn pane_metadata_preserves_real_absence_after_bounded_retries() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let missing = no_active_session();
    let mut host = ScriptedTransport::new([missing.clone(), missing.clone(), missing]);

    let error = runtime.list_panes(&mut host, "repo-main").unwrap_err();

    assert!(error.to_string().contains("There is no active session"));
    assert_eq!(host.commands.len(), 3);
}

#[test]
fn metadata_retries_a_transport_timeout_after_clean_cancellation() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let tabs = r#"[{"tab_id":4,"position":0,"name":"shell","active":true}]"#;
    let mut host = ScriptedTransport::with_results([
        Err(TransportError::CommandTimedOut {
            process_id: 42,
            timeout_ms: 2_000,
            cancellation_error: None,
        }),
        Ok(success(tabs)),
    ]);

    let listed = runtime.list_tabs(&mut host, "repo-main").unwrap();

    assert_eq!(listed[0].tab_id, 4);
    assert_eq!(host.commands.len(), 2);
}

#[test]
fn client_metadata_retries_blank_and_spurious_missing_results() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let clients =
        "CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n9 terminal_4 zellij attach repo-main\n";
    let mut blank = ScriptedTransport::new([success("\n"), success(clients)]);

    assert_eq!(
        runtime.list_clients(&mut blank, "repo-main").unwrap().len(),
        1
    );
    assert_eq!(blank.commands.len(), 2);

    let mut missing = ScriptedTransport::new([missing_session("repo-main"), success(clients)]);
    assert!(runtime
        .session_is_active(&mut missing, "repo-main")
        .unwrap());
    assert_eq!(missing.commands.len(), 2);
}

#[test]
fn client_metadata_preserves_real_absence_after_bounded_retries() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let missing = missing_session("repo-main");
    let mut host = ScriptedTransport::new([missing.clone(), missing.clone(), missing]);

    assert!(!runtime.session_is_active(&mut host, "repo-main").unwrap());
    assert_eq!(host.commands.len(), 3);
}

#[test]
fn metadata_timeout_with_failed_cancellation_is_not_retried() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::with_results([Err(TransportError::CommandTimedOut {
        process_id: 42,
        timeout_ms: 2_000,
        cancellation_error: Some("process group survived".to_owned()),
    })]);

    let error = runtime.list_clients(&mut host, "repo-main").unwrap_err();

    assert!(error.to_string().contains("process group survived"));
    assert_eq!(host.commands.len(), 1);
}

#[test]
fn nonempty_malformed_metadata_fails_without_retrying() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let tabs = r#"[{"tab_id":4,"position":0,"name":"shell","active":true}]"#;
    let mut host = ScriptedTransport::new([success("not-json"), success(tabs)]);

    let error = runtime.list_tabs(&mut host, "repo-main").unwrap_err();

    assert!(error.to_string().contains("invalid tab JSON"));
    assert_eq!(host.commands.len(), 1);
}

#[test]
fn valid_empty_metadata_is_authoritative_without_retrying() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([success("[]")]);

    assert!(runtime
        .list_tabs(&mut host, "repo-main")
        .unwrap()
        .is_empty());
    assert_eq!(host.commands.len(), 1);
}

#[test]
fn blank_metadata_at_the_deadline_fails_closed() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([success("\n")]);

    let error = runtime
        .list_tabs_with_timeout_for_test(&mut host, "repo-main", Duration::ZERO)
        .unwrap_err();

    assert!(error
        .to_string()
        .contains("returned no complete JSON after bounded retries within 0ms"));
    assert_eq!(host.commands.len(), 1);
    assert_eq!(host.timeouts, [Duration::ZERO]);
}

#[test]
fn blank_metadata_stops_after_the_bounded_attempt_count() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([success(""), success("\n"), success("\r\n")]);

    let error = runtime.list_tabs(&mut host, "repo-main").unwrap_err();

    assert!(error
        .to_string()
        .contains("returned no complete JSON after bounded retries"));
    assert_eq!(host.commands.len(), 3);
}

fn metadata_timeout(message: &str) -> CommandOutput {
    CommandOutput {
        success: false,
        status: Some(2),
        stdout: Vec::new(),
        stderr: format!("{message}\n").into_bytes(),
    }
}
