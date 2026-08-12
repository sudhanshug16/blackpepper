use super::*;
use crate::client::HostConnection;
use crate::transport::{ProcessSpec, PtyProcess};
use portable_pty::PtySize;
use ratatui::{backend::TestBackend, Terminal};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant};

fn attached_fixture() -> (
    tempfile::TempDir,
    ClientRuntime,
    ClientState,
    mpsc::Receiver<crate::client::ClientEvent>,
    crate::core::HostId,
    crate::core::WorkspaceId,
) {
    let root = tempfile::tempdir().unwrap();
    let workspace_root = root.path().join("workspace");
    std::fs::create_dir(&workspace_root).unwrap();
    let mut runtime = ClientRuntime::test_fixture(root.path());
    let host_id = runtime.local_host_id();
    let workspace_id = runtime
        .register_workspace(host_id, &workspace_root)
        .unwrap();
    let (event_tx, event_rx) = mpsc::channel();
    let mut state = ClientState::new(
        crate::client_config::load_contents(None, None, None).unwrap(),
        runtime.snapshot().unwrap(),
        event_tx,
    );
    state.connections.insert(host_id, HostConnection::Local);
    state.selected_host = Some(host_id);
    state.selected_workspace = Some(workspace_id);
    let process = PtyProcess::spawn(
        &ProcessSpec::new("sh").args(["-c", "exec sleep 30"]),
        PtySize::default(),
    )
    .unwrap();
    control::apply_attachment(&mut state, workspace_id, process, 1).unwrap();
    state.rebuild_tree();
    (root, runtime, state, event_rx, host_id, workspace_id)
}

fn completion(
    receiver: &mpsc::Receiver<crate::client::ClientEvent>,
) -> (uuid::Uuid, crate::core::HostId, u64) {
    loop {
        match receiver.recv_timeout(Duration::from_secs(3)).unwrap() {
            crate::client::ClientEvent::HostOperationComplete {
                token,
                host_id,
                generation,
            } => return (token, host_id, generation),
            _ => continue,
        }
    }
}

#[test]
fn stalled_initial_focus_keeps_render_responsive_and_gates_input_until_completion() {
    let (_root, mut runtime, mut state, events, host_id, workspace_id) = attached_fixture();
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let worker_entered = Arc::clone(&entered);
    let worker_release = Arc::clone(&release);

    let started = Instant::now();
    schedule_initial_shell_focus_with(&mut state, &mut runtime, host_id, workspace_id, move |_| {
        worker_entered.store(true, Ordering::Release);
        while !worker_release.load(Ordering::Acquire)
            && !crate::transport::CommandCancellation::scope_is_cancelled()
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    })
    .unwrap();
    assert!(started.elapsed() < Duration::from_millis(250));
    while !entered.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    assert_eq!(state.mode, ClientMode::Manage);

    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    let rendered = Instant::now();
    terminal
        .draw(|frame| crate::client::render(&mut state, frame))
        .unwrap();
    assert!(rendered.elapsed() < Duration::from_millis(250));

    release.store(true, Ordering::Release);
    let (token, completed_host, generation) = completion(&events);
    complete(&mut state, &mut runtime, token, completed_host, generation);

    assert_eq!(state.mode, ClientMode::Work);
    assert!(state.output.is_none());
}

#[test]
fn initial_focus_failure_is_visible_and_releases_terminal_input() {
    let (_root, mut runtime, mut state, events, host_id, workspace_id) = attached_fixture();
    schedule_initial_shell_focus_with(&mut state, &mut runtime, host_id, workspace_id, |_| {
        Err("client set changed; no focus change was sent".to_owned())
    })
    .unwrap();

    let (token, completed_host, generation) = completion(&events);
    complete(&mut state, &mut runtime, token, completed_host, generation);

    assert_eq!(state.mode, ClientMode::Work);
    assert!(state
        .output
        .as_deref()
        .is_some_and(|message| message.contains("client set changed; no focus change was sent")));
}
