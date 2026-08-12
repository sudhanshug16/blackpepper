use super::*;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[path = "worktrunk_supervision_fixture.rs"]
mod fixture;
use fixture::SupervisionFixture;

const CHILD_MODE: &str = "BLACKPEPPER_REMOVE_GUARDIAN_TEST_CHILD";

#[test]
fn killed_remove_helper_keeps_lock_until_child_tree_dies_then_reconciles() {
    if std::env::var_os(CHILD_MODE).is_some() {
        run_remove_child();
        return;
    }

    let fixture = SupervisionFixture::new();
    fs::write(&fixture.block_remove, "block\n").unwrap();
    let preview = fixture
        .executor()
        .remove(
            &fixture.registry,
            fixture.target.id,
            &fixture.target.root_path,
            None,
        )
        .unwrap();
    let HostServicePayload::WorktrunkApprovalRequired { approval, .. } = preview else {
        panic!("expected approval preview");
    };
    let mut helper = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "host_services::worktrunk_exec::supervision_tests::killed_remove_helper_keeps_lock_until_child_tree_dies_then_reconciles",
            "--nocapture",
        ])
        .env(CHILD_MODE, "kill")
        .env("BLACKPEPPER_REMOVE_STATE_ROOT", &fixture.state_root)
        .env("BLACKPEPPER_REMOVE_RUNTIME_ROOT", &fixture.runtime_root)
        .env("BLACKPEPPER_REMOVE_BINARY", &fixture.binary)
        .env("BLACKPEPPER_REMOVE_WORKSPACE", fixture.target.id.to_string())
        .env("BLACKPEPPER_REMOVE_TARGET", &fixture.target.root_path)
        .env(
            "BLACKPEPPER_REMOVE_APPROVAL",
            serde_json::to_string(&approval).unwrap(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    wait_until(Duration::from_secs(4), || fixture.ready.exists());
    assert!(fixture
        .registry
        .worktrunk_removal(fixture.target.id)
        .unwrap()
        .is_some());
    let child_pids = fs::read_to_string(&fixture.pids)
        .unwrap()
        .split_whitespace()
        .map(|value| value.parse::<libc::pid_t>().unwrap())
        .collect::<Vec<_>>();

    unsafe {
        libc::kill(helper.id() as libc::pid_t, libc::SIGKILL);
    }
    let _ = helper.wait();

    let started = Instant::now();
    let mut saw_locked = false;
    let acquired = loop {
        match RepositoryLock::acquire(
            &fixture.paths.repository_lock_dir(),
            Path::new(&fixture.survivor.root_path),
        ) {
            Ok(lock) => break lock,
            Err(_) => saw_locked = true,
        }
        assert!(started.elapsed() < Duration::from_secs(4));
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(saw_locked, "orphan child tree was exposed without its lock");
    assert!(
        child_pids.iter().all(|pid| !process_exists(*pid)),
        "a fake wt descendant survived after the repository lock reopened"
    );
    drop(acquired);

    let payload = fixture
        .executor()
        .list(
            &fixture.registry,
            fixture.target.id,
            &fixture.target.root_path,
        )
        .unwrap();
    assert!(matches!(payload, HostServicePayload::Worktrees { .. }));
    assert!(fixture
        .registry
        .workspace(fixture.target.id)
        .unwrap()
        .is_none());
    assert!(fixture
        .registry
        .worktrunk_removal(fixture.target.id)
        .unwrap()
        .is_none());
}

#[test]
fn post_dispatch_execution_error_keeps_removal_marker_for_list_reconciliation() {
    if std::env::var_os(CHILD_MODE).is_some() {
        run_remove_child();
        return;
    }

    let fixture = SupervisionFixture::new();
    let preview = fixture
        .executor()
        .remove(
            &fixture.registry,
            fixture.target.id,
            &fixture.target.root_path,
            None,
        )
        .unwrap();
    let HostServicePayload::WorktrunkApprovalRequired { approval, .. } = preview else {
        panic!("expected approval preview");
    };
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "host_services::worktrunk_exec::supervision_tests::post_dispatch_execution_error_keeps_removal_marker_for_list_reconciliation",
            "--nocapture",
        ])
        .env(CHILD_MODE, "error")
        .env("BLACKPEPPER_REMOVE_STATE_ROOT", &fixture.state_root)
        .env("BLACKPEPPER_REMOVE_RUNTIME_ROOT", &fixture.runtime_root)
        .env("BLACKPEPPER_REMOVE_BINARY", &fixture.binary)
        .env("BLACKPEPPER_REMOVE_WORKSPACE", fixture.target.id.to_string())
        .env("BLACKPEPPER_REMOVE_TARGET", &fixture.target.root_path)
        .env(
            "BLACKPEPPER_REMOVE_APPROVAL",
            serde_json::to_string(&approval).unwrap(),
        )
        .env("BLACKPEPPER_TEST_FAIL_AFTER_EXEC", &fixture.fail_after_exec)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "child did not observe the injected error");
    assert!(fixture
        .registry
        .worktrunk_removal(fixture.target.id)
        .unwrap()
        .is_some());
    assert_eq!(
        fixture
            .registry
            .snapshot()
            .unwrap()
            .pending_worktree_removals,
        vec![fixture.target.id]
    );

    fixture
        .executor()
        .list(
            &fixture.registry,
            fixture.target.id,
            &fixture.target.root_path,
        )
        .unwrap();
    assert!(fixture
        .registry
        .worktrunk_removal(fixture.target.id)
        .unwrap()
        .is_none());
}

fn run_remove_child() {
    let state_root = std::env::var_os("BLACKPEPPER_REMOVE_STATE_ROOT").unwrap();
    let runtime_root = std::env::var_os("BLACKPEPPER_REMOVE_RUNTIME_ROOT").unwrap();
    let paths = CorePaths::from_roots(state_root, runtime_root);
    let registry = HostRegistry::open(paths.registry_path()).unwrap();
    let binary = PathBuf::from(std::env::var_os("BLACKPEPPER_REMOVE_BINARY").unwrap());
    let workspace_id = std::env::var("BLACKPEPPER_REMOVE_WORKSPACE")
        .unwrap()
        .parse()
        .unwrap();
    let target = std::env::var("BLACKPEPPER_REMOVE_TARGET").unwrap();
    let approval =
        serde_json::from_str(&std::env::var("BLACKPEPPER_REMOVE_APPROVAL").unwrap()).unwrap();
    let result = WorktrunkExecutor::with_binary(&paths, binary).remove(
        &registry,
        workspace_id,
        &target,
        Some(&approval),
    );
    if std::env::var(CHILD_MODE).unwrap() == "error" {
        let error = result.unwrap_err();
        assert!(error.contains("result is unknown"));
        assert!(registry.worktrunk_removal(workspace_id).unwrap().is_some());
    }
}

fn process_exists(pid: libc::pid_t) -> bool {
    (unsafe { libc::kill(pid, 0) == 0 })
        || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

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
