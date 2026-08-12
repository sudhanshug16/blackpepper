use super::*;

#[test]
fn captures_fixed_command_without_a_shell() {
    let output = run_bounded(OsStr::new("printf"), ["%s", "hello world"]).unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"hello world");
    assert!(!output.truncated);
}

#[cfg(unix)]
#[test]
fn repository_lock_outlives_killed_helper_until_child_group_is_gone() {
    use super::super::worktrunk_lock::RepositoryLock;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    const CHILD_MODE: &str = "BLACKPEPPER_GUARDIAN_TEST_CHILD";
    if std::env::var_os(CHILD_MODE).is_some() {
        let lock_dir =
            std::path::PathBuf::from(std::env::var_os("BLACKPEPPER_GUARDIAN_LOCK_DIR").unwrap());
        let repository =
            std::path::PathBuf::from(std::env::var_os("BLACKPEPPER_GUARDIAN_REPOSITORY").unwrap());
        let script =
            std::path::PathBuf::from(std::env::var_os("BLACKPEPPER_GUARDIAN_SCRIPT").unwrap());
        let lock = RepositoryLock::acquire(&lock_dir, &repository).unwrap();
        let _ = run_bounded_guarded(&lock, script.as_os_str(), std::iter::empty::<&str>());
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let lock_dir = root.path().join("locks");
    let repository = root.path().join("repo");
    let ready = root.path().join("ready");
    let pids = root.path().join("pids");
    let script = root.path().join("blocking-wt");
    fs::create_dir(&repository).unwrap();
    fs::write(
        &script,
        format!(
            "#!/bin/sh\n\
             trap '' TERM\n\
             (trap '' TERM; while :; do sleep 1; done) &\n\
             printf '%s %s' \"$$\" \"$!\" > '{}'\n\
             : > '{}'\n\
             while :; do sleep 1; done\n",
            pids.display(),
            ready.display(),
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();

    let mut helper = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "host_services::process::tests::repository_lock_outlives_killed_helper_until_child_group_is_gone",
            "--nocapture",
        ])
        .env(CHILD_MODE, "1")
        .env("BLACKPEPPER_GUARDIAN_LOCK_DIR", &lock_dir)
        .env("BLACKPEPPER_GUARDIAN_REPOSITORY", &repository)
        .env("BLACKPEPPER_GUARDIAN_SCRIPT", &script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until(Duration::from_secs(3), || ready.exists());
    let process_ids = fs::read_to_string(&pids)
        .unwrap()
        .split_whitespace()
        .map(|value| value.parse::<libc::pid_t>().unwrap())
        .collect::<Vec<_>>();

    unsafe {
        libc::kill(helper.id() as libc::pid_t, libc::SIGKILL);
    }
    let _ = helper.wait();

    let started = Instant::now();
    let mut observed_guarded_lock = false;
    let acquired = loop {
        match RepositoryLock::acquire(&lock_dir, &repository) {
            Ok(lock) => break lock,
            Err(_) => observed_guarded_lock = true,
        }
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "orphan Worktrunk group was not cleaned up"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        observed_guarded_lock,
        "repository lock was exposed before forced child cleanup"
    );
    assert!(
        process_ids.iter().all(|pid| !process_exists(*pid)),
        "repository lock became available while a Worktrunk child survived"
    );
    drop(acquired);
}

#[cfg(unix)]
#[test]
fn busy_guardian_keeps_lock_after_repository_handle_drops() {
    use super::super::worktrunk_lock::RepositoryLock;
    use std::fs;
    use std::time::{Duration, Instant};

    let root = tempfile::tempdir().unwrap();
    let lock_dir = root.path().join("locks");
    let repository = root.path().join("repo");
    fs::create_dir(&repository).unwrap();
    let lock = RepositoryLock::acquire(&lock_dir, &repository).unwrap();
    lock.hold_guardian_lock_after_drop_for_test().unwrap();
    drop(lock);

    assert!(
        RepositoryLock::acquire(&lock_dir, &repository).is_err(),
        "parent close must not unlock the guardian's shared file description"
    );
    let started = Instant::now();
    loop {
        if let Ok(lock) = RepositoryLock::acquire(&lock_dir, &repository) {
            drop(lock);
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(4));
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
fn process_exists(pid: libc::pid_t) -> bool {
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(unix)]
fn wait_until(timeout: std::time::Duration, mut condition: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if condition() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("condition did not become true before timeout");
}
