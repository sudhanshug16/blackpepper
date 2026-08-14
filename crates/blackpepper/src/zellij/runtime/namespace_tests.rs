use std::path::Path;

use super::{ZellijRuntime, DEVELOPMENT_SOCKET_OVERRIDE};
use crate::zellij::ZellijError;

use super::namespace::candidate_directories_for_test;

mod candidates;
mod support;

use support::*;

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
        missing_named("repo-main"),
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
fn branded_runtime_uses_the_standard_legacy_namespace() {
    let runtime = ZellijRuntime::for_version(
        "/opt/blackpepper-zellij",
        crate::transport::PATCHED_ZELLIJ_VERSION,
    )
    .unwrap();
    let mut host = ScriptedTransport::new([
        metadata("1003", "", "", "", ""),
        missing_socket(),
        missing_socket(),
    ]);

    let (resolved, active) = runtime
        .resolve_session_namespace(&mut host, "bp-workspace-version-hash")
        .unwrap();

    assert!(!active);
    assert_eq!(host.commands[1].args[3], "/tmp/zellij-1003");
    assert_eq!(host.commands[2].args[3], "/run/user/1003/zellij");
    assert_eq!(
        resolved.socket_directory.as_deref(),
        Some("/tmp/zellij-1003")
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
fn absent_session_uses_canonical_namespace_and_forgets_cached_resurrection() {
    let runtime = ZellijRuntime::new("/opt/zellij").unwrap();
    let mut host = ScriptedTransport::new([
        metadata("1003", "relative", "also-relative", "", "relative-native"),
        missing_socket(),
        missing_socket(),
        missing_no_sessions(),
        missing_no_sessions(),
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
