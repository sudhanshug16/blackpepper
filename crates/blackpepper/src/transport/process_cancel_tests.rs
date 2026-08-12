use std::fs;
use std::os::unix::fs::PermissionsExt;

use crate::transport::{ProcessSpec, RunningCommand};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn cancel_forwards_term_before_forcing_the_child() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("term-received");
    let ready = temp.path().join("ready");
    let script = temp.path().join("term-aware");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ntrap ': > \"{}\"; exit 0' TERM\n: > \"{}\"\nwhile :; do sleep 0.05; done\n",
            marker.display(),
            ready.display()
        ),
    )
    .unwrap();
    make_executable(&script);

    let child = RunningCommand::spawn(&ProcessSpec::new(&script), false).unwrap();
    wait_until(Duration::from_secs(1), || ready.exists());
    let started = Instant::now();
    let _ = child.cancel().unwrap();

    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(marker.exists(), "TERM was not delivered before force-kill");
}

#[test]
fn cancel_does_not_wait_for_inherited_output_pipes() {
    let temp = tempfile::tempdir().unwrap();
    let descendant_pid = temp.path().join("descendant-pid");
    let script = temp.path().join("pipe-holder");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ntrap '' TERM\n(sleep 5) &\nprintf '%s' \"$!\" > \"{}\"\nwhile :; do sleep 1; done\n",
            descendant_pid.display()
        ),
    )
    .unwrap();
    make_executable(&script);

    let child = RunningCommand::spawn(&ProcessSpec::new(&script), false).unwrap();
    wait_until(Duration::from_secs(1), || descendant_pid.exists());
    let started = Instant::now();
    let _ = child.cancel().unwrap();

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "cancel waited for a descendant that retained the output pipes"
    );

    if let Ok(pid) = fs::read_to_string(&descendant_pid) {
        if let Ok(pid) = pid.parse::<libc::pid_t>() {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("condition did not become true before timeout");
}

fn make_executable(path: &std::path::Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).unwrap();
}
