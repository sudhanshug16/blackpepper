use std::fs;
use std::os::unix::fs::PermissionsExt;

use crate::agent_status::{IntegrationHealth, Provider};
use crate::zellij::ZellijRuntime;

use super::tests::context;
use super::{
    run_host_local_subscription, run_host_local_subscription_cancellable,
    run_host_local_subscription_cancellable_with_health, run_host_local_subscription_fallible,
    BlockerChange, HostSubscriptionError, ViewportBlockerMonitor,
};

#[test]
fn host_local_runner_emits_only_reduced_transitions() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("fake-zellij");
    fs::write(
        &executable,
        r##"#!/bin/sh
printf '%s\n' '{"event":"pane_update","pane_id":"terminal_7","viewport":["Allow command?","sensitive-host-only-text","Press enter to confirm or esc to cancel"],"scrollback":null,"is_initial":true}'
"##,
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();

    let runtime = ZellijRuntime::new(executable.to_string_lossy()).unwrap();
    let mut monitor = ViewportBlockerMonitor::bundled(
        context(Provider::Codex, IntegrationHealth::Unknown),
        "terminal_7",
    )
    .unwrap();
    let mut transitions = Vec::new();
    let stats = run_host_local_subscription(
        &runtime,
        "workspace-session",
        &mut monitor,
        || 10,
        |transition| transitions.push(transition),
    )
    .unwrap();

    assert_eq!(stats.transitions, 1);
    assert!(matches!(
        transitions[0].state,
        BlockerChange::NeedsInput { .. }
    ));
    let wire = serde_json::to_string(&transitions[0]).unwrap();
    assert!(!wire.contains("sensitive-host-only-text"));
}

#[test]
fn emitter_disconnect_stops_and_reaps_host_subscription() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("fake-zellij");
    fs::write(
        &executable,
        r##"#!/bin/sh
printf '%s\n' '{"event":"pane_update","pane_id":"terminal_7","viewport":["Allow command?","Press enter to confirm or esc to cancel"]}'
exec sleep 30
"##,
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = ZellijRuntime::new(executable.to_string_lossy()).unwrap();
    let mut monitor = ViewportBlockerMonitor::bundled(
        context(Provider::Codex, IntegrationHealth::Unknown),
        "terminal_7",
    )
    .unwrap();

    let started = std::time::Instant::now();
    let error = run_host_local_subscription_fallible(
        &runtime,
        "workspace-session",
        &mut monitor,
        || 10,
        |_| Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "gone")),
    )
    .unwrap_err();
    assert!(matches!(error, HostSubscriptionError::Stream(_)));
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn stdin_cancellation_reaps_a_quiet_subscription() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("fake-zellij");
    fs::write(&executable, "#!/bin/sh\nexec sleep 30\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = ZellijRuntime::new(executable.to_string_lossy()).unwrap();
    let monitor = ViewportBlockerMonitor::bundled(
        context(Provider::Codex, IntegrationHealth::Unknown),
        "terminal_7",
    )
    .unwrap();
    let (cancel_tx, cancel_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        cancel_tx.send(()).unwrap();
    });

    let started = std::time::Instant::now();
    run_host_local_subscription_cancellable(
        &runtime,
        "workspace-session",
        monitor,
        || 10,
        |_| Ok(()),
        cancel_rx,
    )
    .unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn live_health_poll_enables_and_clears_cached_opencode_overlay() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("fake-zellij");
    fs::write(
        &executable,
        r##"#!/bin/sh
printf '%s\n' '{"event":"pane_update","pane_id":"terminal_7","viewport":["Permission required","enter confirm · esc dismiss"]}'
exec sleep 30
"##,
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = ZellijRuntime::new(executable.to_string_lossy()).unwrap();
    let monitor = ViewportBlockerMonitor::bundled(
        context(
            Provider::OpenCode,
            IntegrationHealth::Healthy {
                integration_version: Some(1),
            },
        ),
        "terminal_7",
    )
    .unwrap();
    let (cancel_tx, cancel_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        cancel_tx.send(()).unwrap();
    });
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let health_calls = std::sync::Arc::clone(&calls);
    let transitions = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = std::sync::Arc::clone(&transitions);

    run_host_local_subscription_cancellable_with_health(
        &runtime,
        "workspace-session",
        monitor,
        || 10,
        move |transition| {
            observed.lock().unwrap().push(transition);
            Ok(())
        },
        cancel_rx,
        move || match health_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
            0..=1 => IntegrationHealth::Healthy {
                integration_version: Some(1),
            },
            2..=4 => IntegrationHealth::Stale,
            _ => IntegrationHealth::Healthy {
                integration_version: Some(1),
            },
        },
        std::time::Duration::from_millis(10),
    )
    .unwrap();

    let transitions = transitions.lock().unwrap();
    assert_eq!(transitions.len(), 2);
    assert!(matches!(
        transitions[0].state,
        BlockerChange::NeedsInput { .. }
    ));
    assert_eq!(transitions[1].state, BlockerChange::Cleared);
}
