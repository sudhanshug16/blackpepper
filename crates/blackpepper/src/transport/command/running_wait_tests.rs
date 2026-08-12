use super::*;
use crate::transport::{CommandCancellation, ProcessSpec};

#[test]
fn scoped_wait_drains_more_than_pipe_capacity_from_both_streams() {
    let bytes = 256 * 1024;
    let command = RunningCommand::spawn(
        &ProcessSpec::new("sh").args([
            "-c",
            &format!("head -c {bytes} /dev/zero; head -c {bytes} /dev/zero >&2"),
        ]),
        false,
    )
    .unwrap();

    let output = CommandCancellation::default()
        .scoped(|| command.wait_with_output())
        .unwrap();

    assert!(output.success);
    assert_eq!(output.stdout.len(), bytes);
    assert_eq!(output.stderr.len(), bytes);
}

#[test]
fn scoped_wait_cancels_a_blocked_command_promptly() {
    let command =
        RunningCommand::spawn(&ProcessSpec::new("sh").args(["-c", "exec sleep 30"]), false)
            .unwrap();
    let cancellation = CommandCancellation::default();
    let worker_cancellation = cancellation.clone();
    let worker =
        std::thread::spawn(move || worker_cancellation.scoped(|| command.wait_with_output()));
    std::thread::sleep(Duration::from_millis(30));
    let started = Instant::now();
    cancellation.cancel();
    let error = worker.join().unwrap().unwrap_err();

    assert!(matches!(error, TransportError::CommandCancelled { .. }));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn scoped_bounded_wait_honors_cancellation_before_deadline() {
    let command =
        RunningCommand::spawn(&ProcessSpec::new("sh").args(["-c", "exec sleep 30"]), false)
            .unwrap();
    let cancellation = CommandCancellation::default();
    let worker_cancellation = cancellation.clone();
    let worker = std::thread::spawn(move || {
        worker_cancellation.scoped(|| command.wait_with_output_timeout(Duration::from_secs(15)))
    });
    std::thread::sleep(Duration::from_millis(30));
    let started = Instant::now();
    cancellation.cancel();
    let error = worker.join().unwrap().unwrap_err();

    assert!(matches!(error, TransportError::CommandCancelled { .. }));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn bounded_wait_does_not_wait_for_descendant_pipe_eof_after_parent_exit() {
    let command = RunningCommand::spawn(
        &ProcessSpec::new("sh").args(["-c", "(sleep 3) & printf finished"]),
        false,
    )
    .unwrap();
    let started = Instant::now();

    let output = command
        .wait_with_output_timeout(Duration::from_secs(2))
        .unwrap();

    assert!(output.success);
    assert_eq!(output.stdout, b"finished");
    assert!(started.elapsed() < Duration::from_secs(1));
}
