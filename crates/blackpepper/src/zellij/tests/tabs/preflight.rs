use super::*;

use crate::transport::CommandCancellation;

#[test]
fn ensure_tab_retries_a_one_to_zero_client_snapshot_before_one_mutation() {
    let one = client(9);
    let zero = no_clients();
    let inactive = shell_tabs(false);
    let mut host = ScriptedTransport::new([
        one,
        inactive.clone(),
        zero.clone(),
        zero.clone(),
        inactive,
        zero,
        success("7\n"),
        created_tabs(false),
        ready_terminal_pane(7, "service-api"),
    ]);

    assert_eq!(ensure_service_tab(&mut host).unwrap(), (7, true));
    assert_eq!(new_tab_count(&host), 1);
    assert_eq!(host.commands.len(), 9);
}

#[test]
fn ensure_tab_retries_a_zero_to_one_client_snapshot_and_restores_focus() {
    let zero = no_clients();
    let one = client(9);
    let active_shell = shell_tabs(true);
    let created = created_tabs(true);
    let mut host = ScriptedTransport::new([
        zero,
        active_shell.clone(),
        one.clone(),
        one.clone(),
        active_shell,
        one.clone(),
        success("7\n"),
        created.clone(),
        ready_terminal_pane(7, "service-api"),
        one.clone(),
        created,
        one,
        success(""),
    ]);

    assert_eq!(ensure_service_tab(&mut host).unwrap(), (7, true));
    assert_eq!(new_tab_count(&host), 1);
    assert_eq!(
        wrapped_zellij_args(host.commands.last().unwrap(), "/opt/zellij"),
        ["--session", "repo-main", "action", "go-to-tab-by-id", "0"]
    );
}

#[test]
fn stable_zero_clients_ignore_a_stale_active_tab_marker() {
    let zero = no_clients();
    let mut host = ScriptedTransport::new([
        zero.clone(),
        shell_tabs(true),
        zero,
        success("7\n"),
        created_tabs(false),
        ready_terminal_pane(7, "service-api"),
    ]);

    assert_eq!(ensure_service_tab(&mut host).unwrap(), (7, true));
    assert_eq!(new_tab_count(&host), 1);
    assert_eq!(host.commands.len(), 6);
}

#[test]
fn preflight_deadline_expires_without_a_mutation() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([client(9), shell_tabs(false), no_clients()]);

    let error = runtime
        .background_tab_preflight(&mut host, "repo-main", "service-api", Duration::ZERO)
        .unwrap_err();

    assert!(error.to_string().contains("did not stabilize within 0ms"));
    assert!(host.commands.is_empty());
    assert!(host.timeouts.is_empty());
    assert_eq!(new_tab_count(&host), 0);
}

#[test]
fn cancellation_during_the_final_preflight_read_sends_no_mutation() {
    let cancellation = CommandCancellation::default();
    let mut host = CancelAfterThirdRead {
        inner: ScriptedTransport::new([no_clients(), shell_tabs(false), no_clients()]),
        cancellation: cancellation.clone(),
    };

    let error = cancellation
        .scoped(|| ensure_service_tab(&mut host))
        .unwrap_err();

    assert!(error
        .to_string()
        .contains("preflight was cancelled; no new-tab was sent"));
    assert_eq!(host.inner.commands.len(), 3);
    assert_eq!(new_tab_count(&host.inner), 0);
}

#[test]
fn preflight_refuses_multiple_clients_on_the_second_client_read() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([no_clients(), shell_tabs(false), clients(&[9, 10])]);

    let error = runtime
        .background_tab_preflight(
            &mut host,
            "repo-main",
            "service-api",
            Duration::from_secs(1),
        )
        .unwrap_err();

    assert!(error.to_string().contains("2 controlling client(s)"));
    assert_eq!(host.commands.len(), 3);
    assert_eq!(new_tab_count(&host), 0);
}

#[test]
fn post_create_detach_skips_focus_restoration() {
    let one = client(9);
    let created = created_tabs(true);
    let mut host = ScriptedTransport::new([
        one.clone(),
        shell_tabs(true),
        one.clone(),
        success("7\n"),
        created.clone(),
        ready_terminal_pane(7, "service-api"),
        one,
        created,
        no_clients(),
    ]);

    assert_eq!(ensure_service_tab(&mut host).unwrap(), (7, true));
    assert_eq!(new_tab_count(&host), 1);
    assert_eq!(host.commands.len(), 9);
    assert_eq!(
        wrapped_zellij_args(host.commands.last().unwrap(), "/opt/zellij"),
        ["--session", "repo-main", "action", "list-clients"]
    );
}

#[test]
fn post_create_same_id_client_drift_fails_without_restoring_focus() {
    let original = client(9);
    let created = created_tabs(true);
    let mut host = ScriptedTransport::new([
        original.clone(),
        shell_tabs(true),
        original.clone(),
        success("7\n"),
        created.clone(),
        ready_terminal_pane(7, "service-api"),
        original,
        created,
        client_row(9, "terminal_10", "replacement-client"),
    ]);

    let error = ensure_service_tab(&mut host).unwrap_err();

    assert!(error.to_string().contains("client set changed"));
    assert_eq!(new_tab_count(&host), 1);
    assert_eq!(host.commands.len(), 9);
    assert_eq!(
        wrapped_zellij_args(host.commands.last().unwrap(), "/opt/zellij"),
        ["--session", "repo-main", "action", "list-clients"]
    );
}

fn ensure_service_tab(host: &mut dyn HostTransport) -> Result<(u64, bool), ZellijError> {
    ZellijRuntime::new("/opt/zellij")?.ensure_tab_with_reconcile_timeout(
        host,
        "repo-main",
        "service-api",
        Path::new("/srv/repo"),
        None,
        Duration::from_millis(200),
    )
}

fn no_clients() -> CommandOutput {
    clients(&[])
}

fn client(client_id: u32) -> CommandOutput {
    clients(&[client_id])
}

fn client_row(client_id: u32, pane_id: &str, command: &str) -> CommandOutput {
    success(&format!(
        "CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n{client_id} {pane_id} {command}\n"
    ))
}

fn clients(client_ids: &[u32]) -> CommandOutput {
    let rows = client_ids
        .iter()
        .map(|id| format!("{id} terminal_{id} zellij attach repo-main\n"))
        .collect::<String>();
    success(&format!("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n{rows}"))
}

fn shell_tabs(active: bool) -> CommandOutput {
    success(
        &serde_json::json!([{
            "tab_id": 0,
            "position": 0,
            "name": "shell",
            "active": active
        }])
        .to_string(),
    )
}

fn created_tabs(active: bool) -> CommandOutput {
    success(
        &serde_json::json!([
            {"tab_id": 0, "position": 0, "name": "shell", "active": false},
            {"tab_id": 7, "position": 1, "name": "service-api", "active": active}
        ])
        .to_string(),
    )
}

struct CancelAfterThirdRead {
    inner: ScriptedTransport,
    cancellation: CommandCancellation,
}

impl HostTransport for CancelAfterThirdRead {
    fn kind(&self) -> HostKind {
        self.inner.kind()
    }

    fn spawn_exec(&mut self, command: &HostCommand) -> Result<RunningCommand, TransportError> {
        self.inner.spawn_exec(command)
    }

    fn spawn_exec_with_stdin(
        &mut self,
        command: &HostCommand,
    ) -> Result<RunningCommand, TransportError> {
        self.inner.spawn_exec_with_stdin(command)
    }

    fn attach_pty(
        &mut self,
        command: &HostCommand,
        size: PtySize,
    ) -> Result<PtyProcess, TransportError> {
        self.inner.attach_pty(command, size)
    }

    fn forward_local_port(
        &mut self,
        forward: LocalForward,
    ) -> Result<LocalForward, TransportError> {
        self.inner.forward_local_port(forward)
    }

    fn cancel_local_forward(&mut self, forward: &LocalForward) -> Result<(), TransportError> {
        self.inner.cancel_local_forward(forward)
    }

    fn exec(&mut self, command: &HostCommand) -> Result<CommandOutput, TransportError> {
        let output = self.inner.exec(command);
        if self.inner.commands.len() == 3 {
            self.cancellation.cancel();
        }
        output
    }

    fn exec_timeout(
        &mut self,
        command: &HostCommand,
        timeout: Duration,
    ) -> Result<CommandOutput, TransportError> {
        self.inner.timeouts.push(timeout);
        self.exec(command)
    }
}
