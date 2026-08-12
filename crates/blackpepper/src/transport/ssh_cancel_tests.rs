#[cfg(target_os = "linux")]
use std::io::Write;
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

use super::ssh_cancel::{cancellation_command, wrapped_command};
use uuid::Uuid;

#[test]
fn wrapper_records_identity_without_polluting_command_output() {
    let line = wrapped_command("0123456789abcdef", "exec printf '%s' 'hello world'");

    assert!(line.contains("/tmp/blackpepper-cancel-0123456789abcdef"));
    assert!(line.contains("/proc/$bp_pid/stat"));
    assert!(line.contains("sh -c "));
    assert!(line.contains("exec printf"));
    assert!(line.contains("hello world"));
    assert!(!line.contains("echo "));
}

#[test]
fn cancellation_revalidates_start_time_before_each_signal_phase() {
    let line = cancellation_command("0123456789abcdef");

    assert!(line.matches("[ \"$1\" = \"$bp_start\" ]").count() >= 2);
    assert!(line.contains("kill -TERM \"$bp_pid\""));
    assert!(line.contains("kill -KILL \"$bp_pid\""));
    assert!(line.contains("exit 125"));
}

#[cfg(target_os = "linux")]
#[test]
fn wrapper_preserves_stdin_for_backgrounded_helper() {
    let token = Uuid::new_v4().simple().to_string();
    let line = wrapped_command(&token, "exec cat");
    let mut child = Command::new("sh")
        .args(["-c", &line])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"helper-request\n")
        .unwrap();

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(output.stdout, b"helper-request\n");
    assert!(!std::path::Path::new(&format!("/tmp/blackpepper-cancel-{token}")).exists());
}

#[cfg(target_os = "linux")]
#[test]
fn cancellation_stops_wrapped_process_and_removes_metadata() {
    let token = Uuid::new_v4().simple().to_string();
    let directory = format!("/tmp/blackpepper-cancel-{token}");
    let mut wrapped = Command::new("sh")
        .args(["-c", &wrapped_command(&token, "exec sleep 30")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until(Duration::from_secs(1), || {
        std::path::Path::new(&format!("{directory}/pid")).exists()
    });

    let cancel_status = Command::new("sh")
        .args(["-c", &cancellation_command(&token)])
        .status()
        .unwrap();
    assert!(cancel_status.success());

    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = wrapped.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = wrapped.kill();
            let _ = wrapped.wait();
            panic!("wrapped process survived cancellation");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(!status.success(), "cancelled sleep exited successfully");
    assert!(!std::path::Path::new(&directory).exists());
}

#[cfg(target_os = "linux")]
#[test]
fn cancel_before_start_leaves_tombstone_that_blocks_launch() {
    let token = Uuid::new_v4().simple().to_string();
    let directory = format!("/tmp/blackpepper-cancel-{token}");
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("must-not-run");
    let cancel_status = Command::new("sh")
        .args(["-c", &cancellation_command(&token)])
        .status()
        .unwrap();
    assert_eq!(cancel_status.code(), Some(124));
    assert!(std::path::Path::new(&format!("{directory}/cancel")).exists());

    let original = format!(
        "exec {}",
        shell_words::join([
            "sh",
            "-c",
            ": > \"$1\"; sleep 30",
            "sh",
            &marker.to_string_lossy(),
        ])
    );
    let mut wrapped = Command::new("sh")
        .args(["-c", &wrapped_command(&token, &original)])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if wrapped.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = wrapped.kill();
            let _ = wrapped.wait();
            panic!("tombstoned wrapper did not stop");
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(!marker.exists());
    assert!(!std::path::Path::new(&directory).exists());
}

#[cfg(target_os = "linux")]
fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("condition did not become true before timeout");
}
