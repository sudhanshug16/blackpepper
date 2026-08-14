use crate::transport::CommandOutput;

use super::support::*;
use super::{candidate_directories_for_test, ZellijRuntime};

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
