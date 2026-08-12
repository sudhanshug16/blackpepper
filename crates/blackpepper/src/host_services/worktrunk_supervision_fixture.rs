use super::*;
use crate::core::{RepositoryIdentity, WorkspaceRecord};
use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

pub(super) struct SupervisionFixture {
    pub(super) _root: tempfile::TempDir,
    pub(super) state_root: PathBuf,
    pub(super) runtime_root: PathBuf,
    pub(super) paths: CorePaths,
    pub(super) registry: HostRegistry,
    pub(super) survivor: WorkspaceRecord,
    pub(super) target: WorkspaceRecord,
    pub(super) binary: PathBuf,
    pub(super) ready: PathBuf,
    pub(super) pids: PathBuf,
    pub(super) block_remove: PathBuf,
    pub(super) fail_after_exec: PathBuf,
}

impl SupervisionFixture {
    pub(super) fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let state_root = root.path().join("state");
        let runtime_root = root.path().join("run");
        let paths = CorePaths::from_roots(&state_root, &runtime_root);
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
        run_git(&survivor_path, ["config", "user.name", "Blackpepper"]);
        fs::write(survivor_path.join("README"), "fixture\n").unwrap();
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
        let common = fs::canonicalize(survivor_path.join(".git")).unwrap();
        let identity = RepositoryIdentity::local(host_id, common.to_string_lossy()).unwrap();
        let mut survivor = WorkspaceRecord::new(host_id, survivor_path.to_string_lossy());
        survivor.repository = Some(identity.clone());
        let mut target = WorkspaceRecord::new(host_id, target_path.to_string_lossy());
        target.repository = Some(identity);
        registry.upsert_workspace(&survivor).unwrap();
        registry.upsert_workspace(&target).unwrap();

        let list_json = root.path().join("list.json");
        write_list(&list_json, &[&survivor.root_path]);
        let ready = root.path().join("remove-started");
        let pids = root.path().join("remove-pids");
        let block_remove = root.path().join("block-remove");
        let fail_after_exec = root.path().join("fail-after-exec");
        let binary = root.path().join("wt");
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\n\
                 if [ \"$1\" = \"--version\" ]; then printf 'wt 0.72.0\\n'; exit 0; fi\n\
                 if [ \"$3\" = \"config\" ]; then printf '%s' '{{\"state\":\"no_commands\",\"commands\":[],\"stale\":[]}}'; exit 0; fi\n\
                 if [ \"$5\" = \"list\" ]; then cat '{}'; exit 0; fi\n\
                 if [ \"$3\" = \"remove\" ]; then\n\
                   git -C '{}' worktree remove '{}' || exit 90\n\
                   if [ -f '{}' ]; then\n\
                     trap '' TERM\n\
                     (trap '' TERM; while :; do sleep 1; done) &\n\
                     printf '%s %s' \"$$\" \"$!\" > '{}'\n\
                     : > '{}'\n\
                     while :; do sleep 1; done\n\
                   fi\n\
                   : > '{}'\n\
                   exit 0\n\
                 fi\n\
                 exit 91\n",
                list_json.display(),
                survivor_path.display(),
                target_path.display(),
                block_remove.display(),
                pids.display(),
                ready.display(),
                fail_after_exec.display(),
            ),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            _root: root,
            state_root,
            runtime_root,
            paths,
            registry,
            survivor,
            target,
            binary,
            ready,
            pids,
            block_remove,
            fail_after_exec,
        }
    }

    pub(super) fn executor(&self) -> WorktrunkExecutor {
        WorktrunkExecutor::with_binary(&self.paths, self.binary.clone())
    }
}

fn run_git<'a>(cwd: &Path, args: impl IntoIterator<Item = &'a str>) {
    assert!(Command::new("git")
        .current_dir(cwd)
        .args(args)
        .status()
        .unwrap()
        .success());
}

fn write_list(path: &Path, worktrees: &[&str]) {
    let items = worktrees
        .iter()
        .map(|path| json!({"branch": "fixture", "worktree": {"path": path}}))
        .collect::<Vec<_>>();
    fs::write(
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
