use super::*;
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn remote_command_quotes_every_shell_word() {
    let command = HostCommand::new("printf")
        .args(["%s\n", "a value", "$(touch /tmp/nope)"])
        .cwd("/tmp/a directory")
        .env("SAFE_VALUE", "hello world");

    assert_eq!(
        command.remote_shell_line().unwrap(),
        "cd '/tmp/a directory' && exec env 'SAFE_VALUE=hello world' printf '%s\n' 'a value' '$(touch /tmp/nope)'"
    );
}

#[test]
fn invalid_environment_key_is_rejected() {
    let command = HostCommand::new("true").env("BAD-NAME", "value");
    assert!(matches!(
        command.validate(),
        Err(TransportError::InvalidEnvironment(_))
    ));
}

#[test]
fn bounded_wait_returns_fast_output() {
    let command = RunningCommand::spawn(
        &ProcessSpec::new("sh").args(["-c", "printf bounded"]),
        false,
    )
    .unwrap();

    let output = command
        .wait_with_output_timeout(Duration::from_secs(1))
        .unwrap();

    assert!(output.success);
    assert_eq!(output.stdout, b"bounded");
}

#[test]
fn bounded_wait_cancels_a_stuck_command() {
    let command =
        RunningCommand::spawn(&ProcessSpec::new("sh").args(["-c", "exec sleep 30"]), false)
            .unwrap();
    let started = Instant::now();

    let error = command
        .wait_with_output_timeout(Duration::from_millis(40))
        .unwrap_err();

    assert!(matches!(error, TransportError::CommandTimedOut { .. }));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "timed command was not cancelled within the transport bound"
    );
}

#[cfg(unix)]
#[test]
fn private_process_group_cancellation_reaps_observation_descendants() {
    let temp = tempfile::tempdir().unwrap();
    let child_pid_path = temp.path().join("child-pid");
    let command = RunningCommand::spawn_in_process_group(
        &ProcessSpec::new("sh").args([
            "-c".into(),
            "sleep 30 & printf '%s' \"$!\" > \"$1\"; wait".into(),
            "bp-periodic-test".into(),
            child_pid_path.as_os_str().to_owned(),
        ]),
        false,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while !child_pid_path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    let child_pid = std::fs::read_to_string(&child_pid_path)
        .unwrap()
        .parse::<libc::pid_t>()
        .unwrap();

    let _ = command.cancel();

    let deadline = Instant::now() + Duration::from_secs(1);
    while process_is_live(child_pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!process_is_live(child_pid));
}

#[cfg(unix)]
fn process_is_live(pid: libc::pid_t) -> bool {
    let path = format!("/proc/{pid}/stat");
    match std::fs::read_to_string(path) {
        Ok(stat) => !stat
            .rsplit_once(") ")
            .and_then(|(_, suffix)| suffix.chars().next())
            .is_some_and(|state| state == 'Z'),
        Err(_) => false,
    }
}
