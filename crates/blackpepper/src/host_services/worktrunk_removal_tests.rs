use super::*;
use crate::core::{RepositoryIdentity, WorkspaceRecord, WorktrunkRemovalIntent};
use serde_json::json;
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
struct RemovalFixture {
    _root: tempfile::TempDir,
    paths: CorePaths,
    registry: HostRegistry,
    survivor: WorkspaceRecord,
    target: WorkspaceRecord,
    binary: PathBuf,
    list_json: PathBuf,
    remove_marker: PathBuf,
    fail_remove: PathBuf,
}

#[cfg(unix)]
impl RemovalFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
        paths.prepare().unwrap();
        let mut registry = HostRegistry::open(paths.registry_path()).unwrap();
        let host_id = registry.ensure_local_host("test-host").unwrap();
        let survivor_path = root.path().join("repo");
        let target_path = root.path().join("feature");
        run_git(root.path(), ["init", "-q", survivor_path.to_str().unwrap()]);
        run_git(
            &survivor_path,
            ["config", "user.email", "blackpepper@example.invalid"],
        );
        run_git(&survivor_path, ["config", "user.name", "Blackpepper Test"]);
        std::fs::write(survivor_path.join("README"), "fixture\n").unwrap();
        run_git(&survivor_path, ["add", "README"]);
        run_git(&survivor_path, ["commit", "-q", "-m", "fixture"]);
        run_git(
            &survivor_path,
            [
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                target_path.to_str().unwrap(),
            ],
        );
        // macOS exposes temporary directories through `/var` while Git reports
        // their physical `/private/var` paths. Keep every side of the fixture
        // on the same canonical identity, just as production registration does.
        let survivor_path = std::fs::canonicalize(survivor_path).unwrap();
        let target_path = std::fs::canonicalize(target_path).unwrap();

        let common_dir = std::fs::canonicalize(survivor_path.join(".git")).unwrap();
        let identity = RepositoryIdentity::local(host_id, common_dir.to_string_lossy()).unwrap();
        let mut survivor =
            WorkspaceRecord::new(host_id, survivor_path.to_string_lossy().into_owned());
        survivor.repository = Some(identity.clone());
        let mut target = WorkspaceRecord::new(host_id, target_path.to_string_lossy().into_owned());
        target.repository = Some(identity);
        registry.upsert_workspace(&survivor).unwrap();
        registry.upsert_workspace(&target).unwrap();

        let list_json = root.path().join("list.json");
        write_list(&list_json, &[&survivor.root_path, &target.root_path]);
        let remove_marker = root.path().join("remove-ran");
        let fail_remove = root.path().join("fail-remove");
        let binary = root.path().join("wt");
        let script = format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then printf 'wt 0.72.0\\n'; exit 0; fi\n\
             if [ \"$3\" = \"config\" ]; then printf '%s' '{{\"state\":\"no_commands\",\"commands\":[],\"stale\":[]}}'; exit 0; fi\n\
             if [ \"$5\" = \"list\" ]; then cat '{}'; exit 0; fi\n\
             if [ \"$3\" = \"remove\" ]; then touch '{}'; test ! -f '{}' || exit 92; exit 0; fi\n\
             exit 91\n",
            list_json.display(),
            remove_marker.display(),
            fail_remove.display(),
        );
        std::fs::write(&binary, script).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            _root: root,
            paths,
            registry,
            survivor,
            target,
            binary,
            list_json,
            remove_marker,
            fail_remove,
        }
    }

    fn executor(&self) -> WorktrunkExecutor {
        WorktrunkExecutor::with_binary(&self.paths, self.binary.clone())
    }

    fn journal(&self) -> WorktrunkRemovalIntent {
        let key = reconcile::repository_key(Path::new(&self.survivor.root_path)).unwrap();
        let intent = self
            .registry
            .plan_worktrunk_removal(
                self.target.id,
                self.survivor.id,
                &self.target.root_path,
                key,
            )
            .unwrap();
        self.registry.journal_worktrunk_removal(&intent).unwrap();
        intent
    }
}

#[cfg(unix)]
#[path = "worktrunk_lifecycle_tests.rs"]
mod lifecycle;

#[cfg(unix)]
#[test]
fn successful_remove_owns_shared_registry_cleanup() {
    let fixture = RemovalFixture::new();
    let executor = fixture.executor();
    let preview = executor
        .remove(
            &fixture.registry,
            fixture.target.id,
            &fixture.target.root_path,
            None,
        )
        .unwrap();
    let HostServicePayload::WorktrunkApprovalRequired {
        approval, command, ..
    } = preview
    else {
        panic!("expected approval preview");
    };
    assert!(command.contains(&fixture.target.root_path));
    assert!(!command.contains("--force"));

    let result = executor
        .remove(
            &fixture.registry,
            fixture.target.id,
            &fixture.target.root_path,
            Some(&approval),
        )
        .unwrap();
    assert!(matches!(
        result,
        HostServicePayload::WorktrunkMutation {
            outcome: WorktrunkMutationOutcome::Removed { ref path }
        } if path == Path::new(&fixture.target.root_path)
    ));
    assert!(fixture.remove_marker.exists());
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

#[cfg(unix)]
#[test]
fn list_reconciles_a_crash_after_remove_without_retrying_remove() {
    let fixture = RemovalFixture::new();
    let intent = fixture.journal();
    run_git(
        Path::new(&fixture.survivor.root_path),
        ["worktree", "remove", &fixture.target.root_path],
    );
    write_list(&fixture.list_json, &[&fixture.survivor.root_path]);

    let executor = fixture.executor();
    let payload = executor
        .list(
            &fixture.registry,
            fixture.target.id,
            &fixture.target.root_path,
        )
        .unwrap();
    assert!(matches!(payload, HostServicePayload::Worktrees { .. }));
    assert!(
        !fixture.remove_marker.exists(),
        "Worktrunk remove was retried"
    );
    assert!(fixture
        .registry
        .workspace(fixture.target.id)
        .unwrap()
        .is_none());
    assert!(fixture
        .registry
        .worktrunk_removal(intent.workspace_id)
        .unwrap()
        .is_none());

    let second = executor
        .list(
            &fixture.registry,
            fixture.survivor.id,
            &fixture.survivor.root_path,
        )
        .unwrap();
    assert!(matches!(second, HostServicePayload::Worktrees { .. }));
    assert!(!fixture.remove_marker.exists());
}

#[cfg(unix)]
#[test]
fn list_clears_the_marker_without_deleting_a_still_present_target() {
    let fixture = RemovalFixture::new();
    fixture.journal();
    let executor = fixture.executor();
    executor
        .list(
            &fixture.registry,
            fixture.survivor.id,
            &fixture.survivor.root_path,
        )
        .unwrap();

    assert!(fixture
        .registry
        .workspace(fixture.target.id)
        .unwrap()
        .is_some());
    assert!(fixture
        .registry
        .worktrunk_removal(fixture.target.id)
        .unwrap()
        .is_none());
    assert!(!fixture.remove_marker.exists());
}

#[cfg(unix)]
#[test]
fn remove_rejects_a_stale_client_path_before_preview() {
    let fixture = RemovalFixture::new();
    let error = fixture
        .executor()
        .remove(
            &fixture.registry,
            fixture.target.id,
            "/stale/client/path",
            None,
        )
        .unwrap_err();
    assert!(error.contains("does not match"));
    assert!(!fixture.remove_marker.exists());
}

#[cfg(unix)]
#[test]
fn dispatched_failure_requires_list_before_any_explicit_retry() {
    let fixture = RemovalFixture::new();
    let executor = fixture.executor();
    let preview = executor
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
    std::fs::write(&fixture.fail_remove, "fail\n").unwrap();
    let error = executor
        .remove(
            &fixture.registry,
            fixture.target.id,
            &fixture.target.root_path,
            Some(&approval),
        )
        .unwrap_err();
    assert!(error.contains("not retried"));
    assert!(fixture
        .registry
        .worktrunk_removal(fixture.target.id)
        .unwrap()
        .is_some());

    let retry = executor
        .remove(
            &fixture.registry,
            fixture.target.id,
            &fixture.target.root_path,
            Some(&approval),
        )
        .unwrap_err();
    assert!(retry.contains("previous Worktrunk removal"));
    executor
        .list(
            &fixture.registry,
            fixture.survivor.id,
            &fixture.survivor.root_path,
        )
        .unwrap();
    assert!(fixture
        .registry
        .workspace(fixture.target.id)
        .unwrap()
        .is_some());
    assert!(fixture
        .registry
        .worktrunk_removal(fixture.target.id)
        .unwrap()
        .is_none());
}

#[cfg(unix)]
fn run_git<'a>(cwd: &Path, args: impl IntoIterator<Item = &'a str>) {
    let status = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success());
}

#[cfg(unix)]
fn write_list(path: &Path, worktrees: &[&str]) {
    let items = worktrees
        .iter()
        .map(|path| json!({"branch": "fixture", "worktree": {"path": path}}))
        .collect::<Vec<_>>();
    std::fs::write(
        path,
        serde_json::to_vec(&json!({
            "schema": 2,
            "repo": {"default_branch": "main"},
            "items": items
        }))
        .unwrap(),
    )
    .unwrap();
}
