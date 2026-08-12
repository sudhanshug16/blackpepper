use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;

use portable_pty::PtySize;

use crate::transport::{
    CommandOutput, HostCommand, HostKind, HostTransport, LocalForward, PtyProcess, RunningCommand,
    TransportError,
};

use super::{ZellijRuntime, DEVELOPMENT_SOCKET_OVERRIDE};
use crate::zellij::ZellijError;

use super::namespace::candidate_directories_for_test;

#[test]
fn resolver_adopts_the_only_live_legacy_namespace() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([
        metadata("1003", "", "/custom/runtime", "", ""),
        missing_socket(),
        physical_directory("/run/user/1003/zellij"),
        active_session(),
        physical_directory("/custom/runtime/zellij"),
        missing_named("repo-main"),
        active_session(),
    ]);

    let (resolved, active) = runtime
        .resolve_session_namespace(&mut host, "repo-main")
        .unwrap();
    assert!(active);
    resolved.list_clients(&mut host, "repo-main").unwrap();

    assert_eq!(host.commands[1].args[3], "/tmp/zellij-1003");
    assert_eq!(host.commands[2].args[3], "/run/user/1003/zellij");
    assert_eq!(socket_directory(&host.commands[3]), "/run/user/1003/zellij");
    assert_eq!(host.commands[4].args[3], "/custom/runtime/zellij");
    assert_eq!(
        socket_directory(host.commands.last().unwrap()),
        "/run/user/1003/zellij"
    );
}

#[test]
fn resolver_fails_closed_when_two_namespaces_claim_the_session() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([
        metadata("1003", "", "", "", "/legacy/custom"),
        physical_directory("/tmp/zellij-1003"),
        active_session(),
        physical_directory("/run/user/1003/zellij"),
        active_session(),
        missing_socket(),
    ]);

    let error = runtime
        .resolve_session_namespace(&mut host, "repo-main")
        .unwrap_err();
    assert!(matches!(
        error,
        ZellijError::AmbiguousSessionNamespace { .. }
    ));
    let message = error.to_string();
    assert!(message.contains("/tmp/zellij-1003"));
    assert!(message.contains("/run/user/1003/zellij"));
}

#[test]
fn resolver_adopts_an_inherited_native_legacy_namespace() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([
        metadata("1003", "", "", "", "/legacy/native"),
        missing_socket(),
        missing_socket(),
        physical_directory("/legacy/native"),
        active_session(),
    ]);

    let (resolved, active) = runtime
        .resolve_session_namespace(&mut host, "repo-main")
        .unwrap();

    assert!(active);
    assert_eq!(host.commands[1].args[3], "/tmp/zellij-1003");
    assert_eq!(host.commands[3].args[3], "/legacy/native");
    assert_eq!(socket_directory(&host.commands[4]), "/legacy/native");
    assert_eq!(resolved.socket_directory.as_deref(), Some("/legacy/native"));
}

#[test]
fn namespace_resolution_preserves_the_selected_configuration() {
    let runtime = ZellijRuntime::new("/opt/zellij")
        .unwrap()
        .with_config_file("/var/lib/blackpepper/zellij/config.kdl")
        .unwrap();
    let mut host = ScriptedTransport::new([
        metadata("1003", "", "", "", ""),
        missing_socket(),
        missing_socket(),
    ]);

    let (resolved, active) = runtime
        .resolve_session_namespace(&mut host, "repo-main")
        .unwrap();

    assert!(!active);
    assert_eq!(
        resolved.config_file.as_deref(),
        Some("/var/lib/blackpepper/zellij/config.kdl")
    );
}

#[test]
fn resolver_adopts_the_hosts_legacy_temp_directory() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([
        metadata("1003", "/private/var/tmp", "", "", ""),
        missing_socket(),
        physical_directory("/private/var/tmp/zellij-1003"),
        active_session(),
        missing_socket(),
    ]);

    let (resolved, active) = runtime
        .resolve_session_namespace(&mut host, "repo-main")
        .unwrap();

    assert!(active);
    assert_eq!(
        resolved.socket_directory.as_deref(),
        Some("/private/var/tmp/zellij-1003")
    );
}

#[test]
fn physical_aliases_count_one_live_server_once() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([
        metadata("1003", "", "", "", "/legacy/alias"),
        physical_directory("/real/zellij-1003"),
        active_session(),
        missing_socket(),
        physical_directory("/real/zellij-1003"),
    ]);

    let (resolved, active) = runtime
        .resolve_session_namespace(&mut host, "repo-main")
        .unwrap();

    assert!(active);
    assert_eq!(
        resolved.socket_directory.as_deref(),
        Some("/real/zellij-1003")
    );
    assert_eq!(host.outputs.len(), 0);
}

#[test]
fn unusable_legacy_root_does_not_hide_a_live_canonical_session() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([
        metadata("1003", "", "", "", "/dev/null"),
        physical_directory("/tmp/zellij-1003"),
        active_session(),
        missing_socket(),
        missing_socket(),
    ]);

    let (resolved, active) = runtime
        .resolve_session_namespace(&mut host, "repo-main")
        .unwrap();

    assert!(active);
    assert_eq!(
        resolved.socket_directory.as_deref(),
        Some("/tmp/zellij-1003")
    );
}

#[test]
fn development_override_is_the_exclusive_test_namespace() {
    let candidates = candidate_directories_for_test(
        "1003",
        None,
        Some("/legacy/xdg"),
        Some("/isolated/e2e"),
        Some("/legacy/native"),
        true,
    )
    .unwrap();

    assert_eq!(candidates, ["/isolated/e2e"]);
}

#[test]
fn production_ignores_the_internal_test_override() {
    let candidates = candidate_directories_for_test(
        "1003",
        None,
        None,
        Some("/isolated/e2e"),
        Some("/legacy/native"),
        false,
    )
    .unwrap();

    assert_eq!(
        candidates,
        [
            "/tmp/zellij-1003",
            "/run/user/1003/zellij",
            "/legacy/native"
        ]
    );
}

#[test]
fn candidates_include_host_temp_fallback_and_normalize_aliases() {
    let candidates = candidate_directories_for_test(
        "1003",
        Some("/private/var/folders/user//"),
        Some("/run/user/1003/./"),
        None,
        Some("/tmp/zellij-1003/"),
        false,
    )
    .unwrap();

    assert_eq!(
        candidates,
        [
            "/tmp/zellij-1003",
            "/private/var/folders/user/zellij-1003",
            "/run/user/1003/zellij"
        ]
    );
}

#[test]
fn nul_framing_keeps_malformed_legacy_values_out_of_other_fields() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut metadata =
        b"1003\0/tmp\nmalformed\0relative-xdg\0ignored\nproduction\0relative-native\0".to_vec();
    metadata[0] = b'1';
    let mut host = ScriptedTransport::new([
        CommandOutput {
            success: true,
            status: Some(0),
            stdout: metadata,
            stderr: Vec::new(),
        },
        missing_socket(),
        missing_socket(),
    ]);

    let (resolved, active) = runtime
        .resolve_session_namespace(&mut host, "repo-main")
        .unwrap();

    assert!(!active);
    assert_eq!(
        resolved.socket_directory.as_deref(),
        Some("/tmp/zellij-1003")
    );
}

#[test]
fn absent_session_uses_canonical_namespace_and_forgets_cached_resurrection() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([
        metadata("1003", "relative", "also-relative", "", "relative-native"),
        missing_socket(),
        missing_socket(),
        missing_no_sessions(),
        success(""),
    ]);
    let (runtime, active) = runtime
        .resolve_session_namespace(&mut host, "repo-main")
        .unwrap();
    assert!(!active);
    let environment = std::collections::BTreeMap::from([
        ("BASH_ENV".to_owned(), "/hostile/startup".to_owned()),
        ("PATH".to_owned(), "/hostile".to_owned()),
        ("ZELLIJ_SOCKET_DIR".to_owned(), "/hostile/native".to_owned()),
        (
            DEVELOPMENT_SOCKET_OVERRIDE.to_owned(),
            "/hostile/internal".to_owned(),
        ),
    ]);
    assert!(runtime
        .ensure_session_with_env(&mut host, "repo-main", Path::new("/srv/repo"), &environment)
        .unwrap());

    let create = host.commands.last().unwrap();
    assert_eq!(socket_directory(create), "/tmp/zellij-1003");
    assert_eq!(create.program, "/opt/zellij");
    assert_eq!(
        zellij_arguments(create),
        ["attach", "--create-background", "--forget", "repo-main"]
    );
    assert!(!create.env.contains_key(DEVELOPMENT_SOCKET_OVERRIDE));
    assert_eq!(create.env.get("PATH").map(String::as_str), Some("/hostile"));
    assert_eq!(
        create.env.get("BASH_ENV").map(String::as_str),
        Some("/hostile/startup")
    );
    assert_eq!(
        create.env.get("ZELLIJ_SOCKET_DIR").map(String::as_str),
        Some("/tmp/zellij-1003")
    );
}

fn metadata(uid: &str, temporary: &str, xdg: &str, internal: &str, native: &str) -> CommandOutput {
    success(&format!(
        "{uid}\0{temporary}\0{xdg}\0{internal}\0{native}\0"
    ))
}

fn active_session() -> CommandOutput {
    success("CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n")
}

fn missing_no_sessions() -> CommandOutput {
    CommandOutput {
        success: false,
        status: Some(1),
        stdout: Vec::new(),
        stderr: b"There is no active session!\n".to_vec(),
    }
}

fn missing_socket() -> CommandOutput {
    CommandOutput {
        success: false,
        status: Some(3),
        stdout: Vec::new(),
        stderr: Vec::new(),
    }
}

fn physical_directory(path: &str) -> CommandOutput {
    success(&format!("{path}\n"))
}

fn missing_named(session: &str) -> CommandOutput {
    CommandOutput {
        success: true,
        status: Some(0),
        stdout: b"some-other-session\n".to_vec(),
        stderr: format!("Session '{session}' not found. The following sessions are active:\n")
            .into_bytes(),
    }
}

fn success(stdout: &str) -> CommandOutput {
    CommandOutput {
        success: true,
        status: Some(0),
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
    }
}

fn socket_directory(command: &HostCommand) -> &str {
    command
        .env
        .get("ZELLIJ_SOCKET_DIR")
        .expect("resolved commands pin the socket directory")
}

fn zellij_arguments(command: &HostCommand) -> &[String] {
    &command.args
}

struct ScriptedTransport {
    outputs: VecDeque<CommandOutput>,
    commands: Vec<HostCommand>,
}

impl ScriptedTransport {
    fn new(outputs: impl IntoIterator<Item = CommandOutput>) -> Self {
        Self {
            outputs: outputs.into_iter().collect(),
            commands: Vec::new(),
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
        _size: PtySize,
    ) -> Result<PtyProcess, TransportError> {
        Err(TransportError::Unsupported("not used by this test"))
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
        _timeout: Duration,
    ) -> Result<CommandOutput, TransportError> {
        self.exec(command)
    }
}
