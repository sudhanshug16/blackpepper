use super::init::{ensure_gitignore_entries, ensure_project_config};
use super::workspace::{
    pick_unused_animal_name, unique_animal_names, workspace_create, workspace_destroy,
};
use super::{run_command, CommandContext, CommandSource};
use crate::config::workspace_config_path;
use crate::git::run_git;
use std::collections::HashSet;
use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use tempfile::TempDir;

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

struct EnvVarGuard {
    key: &'static str,
    value: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: String) -> Self {
        let previous = env::var(key).ok();
        env::set_var(key, value);
        Self {
            key,
            value: previous,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(value) = &self.value {
            env::set_var(self.key, value);
        } else {
            env::remove_var(self.key);
        }
    }
}

fn run_git_cmd(args: &[&str], cwd: &Path) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .status()
        .expect("run git");
    assert!(status.success(), "git {:?} failed", args);
}

fn init_repo() -> TempDir {
    let repo = TempDir::new().expect("temp repo");
    run_git_cmd(&["init"], repo.path());
    fs::write(repo.path().join("README.md"), "hello").expect("write file");
    run_git_cmd(&["add", "."], repo.path());
    run_git_cmd(&["commit", "-m", "init"], repo.path());
    repo
}

#[test]
fn gitignore_entries_are_appended_once() {
    let dir = TempDir::new().expect("temp dir");
    let gitignore = dir.path().join(".gitignore");
    fs::write(&gitignore, "target/\n").expect("write gitignore");

    let changed = ensure_gitignore_entries(
        &gitignore,
        &[".blackpepper/workspaces/", ".config/blackpepper/"],
    )
    .expect("update gitignore");
    assert!(changed);

    let contents = fs::read_to_string(&gitignore).expect("read gitignore");
    assert!(contents.contains("target/"));
    assert!(contents.contains(".blackpepper/workspaces/"));
    assert!(contents.contains(".config/blackpepper/"));

    let changed_again = ensure_gitignore_entries(
        &gitignore,
        &[".blackpepper/workspaces/", ".config/blackpepper/"],
    )
    .expect("update gitignore");
    assert!(!changed_again);
}

#[test]
fn project_config_is_created_once() {
    let dir = TempDir::new().expect("temp dir");
    let config_path = dir
        .path()
        .join(".config")
        .join("blackpepper")
        .join("config.toml");

    let created = ensure_project_config(&config_path).expect("create config");
    assert!(created);
    assert!(config_path.exists());

    let created_again = ensure_project_config(&config_path).expect("create config");
    assert!(!created_again);
}

#[test]
fn unique_animal_names_are_valid_and_unique() {
    let names = unique_animal_names();
    let set: HashSet<_> = names.iter().collect();
    assert_eq!(set.len(), names.len());
    assert!(!names.is_empty());
}

#[test]
fn pick_unused_returns_none_when_exhausted() {
    let names = unique_animal_names();
    let used: HashSet<String> = names.into_iter().collect();
    let picked = pick_unused_animal_name(&used);
    assert!(picked.is_none());
}

#[test]
fn workspace_create_and_destroy_workflow() {
    let repo = init_repo();
    let workspace_root = Path::new(".blackpepper/workspaces");
    let config_path = workspace_config_path(repo.path());
    fs::create_dir_all(config_path.parent().expect("config dir")).expect("config dir");
    let tmux_stub = if cfg!(windows) {
        repo.path().join("tmux_stub.cmd")
    } else {
        repo.path().join("tmux_stub.sh")
    };
    let stub_contents = if cfg!(windows) {
        "@echo off\r\nexit /b 0\r\n"
    } else {
        "#!/bin/sh\nexit 0\n"
    };
    fs::write(&tmux_stub, stub_contents).expect("write tmux stub");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&tmux_stub).expect("stat tmux stub").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&tmux_stub, perms).expect("chmod tmux stub");
    }
    let tmux_config = format!("[tmux]\ncommand = \"{}\"\n", tmux_stub.display());
    fs::write(&config_path, tmux_config).expect("write config");
    let ctx = CommandContext {
        cwd: repo.path().to_path_buf(),
        repo_root: Some(repo.path().to_path_buf()),
        workspace_root: workspace_root.to_path_buf(),
        source: CommandSource::Cli,
    };

    let name = "otter";
    let create = workspace_create(&[name.to_string()], &ctx);
    assert!(create.ok, "create failed: {}", create.message);
    assert_eq!(create.data.as_deref(), Some(name));

    let workspace_path = repo.path().join(workspace_root).join(name);
    assert!(workspace_path.exists());

    let destroy = workspace_destroy(&[name.to_string()], &ctx);
    assert!(destroy.ok, "destroy failed: {}", destroy.message);
    assert!(!workspace_path.exists());

    let result = run_git(
        ["show-ref", "--verify", "--quiet", "refs/heads/otter"].as_ref(),
        repo.path(),
    );
    assert!(!result.ok);
}

#[test]
#[cfg(unix)]
fn pr_create_uses_agent_command_and_gh() {
    let _guard = env_lock();
    let repo = init_repo();
    let config_path = workspace_config_path(repo.path());
    fs::create_dir_all(config_path.parent().expect("config dir")).expect("config dir");

    let command = r#"printf '%b' '<pr>\n  <title>feat(pr): stub create</title>\n  <description>\n## Summary\nStubbed PR description\n  </description>\n</pr>\n' # {{PROMPT}}"#;
    let config = format!("[agent]\nprovider = \"unknown\"\ncommand = \"\"\"{command}\"\"\"\n");
    fs::write(&config_path, config).expect("write config");

    let bin_dir = TempDir::new().expect("temp bin");
    let gh_args_path = repo.path().join("gh_args.txt");
    let gh_path = bin_dir.path().join("gh");
    let gh_script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\necho \"https://example.com/pr/123\"\n",
        gh_args_path.display()
    );
    fs::write(&gh_path, gh_script).expect("write gh stub");
    let mut perms = fs::metadata(&gh_path).expect("stat gh").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&gh_path, perms).expect("chmod gh");

    let original_path = env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.path().display(), original_path);
    let _path_guard = EnvVarGuard::set("PATH", new_path);

    let ctx = CommandContext {
        cwd: repo.path().to_path_buf(),
        repo_root: Some(repo.path().to_path_buf()),
        workspace_root: Path::new(".blackpepper/workspaces").to_path_buf(),
        source: CommandSource::Cli,
    };
    let result = run_command("pr", &[String::from("create")], &ctx);
    assert!(result.ok, "pr create failed: {}", result.message);
    assert!(result.message.contains("https://example.com/pr/123"));

    let args = fs::read_to_string(&gh_args_path).expect("read gh args");
    assert!(args.contains("pr"));
    assert!(args.contains("create"));
    assert!(args.contains("--title"));
    assert!(args.contains("feat(pr): stub create"));
    assert!(args.contains("--body"));
    assert!(args.contains("## Summary"));
}

#[test]
#[cfg(unix)]
fn pr_create_surfaces_agent_failure() {
    let _guard = env_lock();
    let repo = init_repo();
    let config_path = workspace_config_path(repo.path());
    fs::create_dir_all(config_path.parent().expect("config dir")).expect("config dir");

    let command = "echo agent_boom 1>&2; exit 42";
    let config = format!("[agent]\nprovider = \"unknown\"\ncommand = \"{command}\"\n");
    fs::write(&config_path, config).expect("write config");

    let ctx = CommandContext {
        cwd: repo.path().to_path_buf(),
        repo_root: Some(repo.path().to_path_buf()),
        workspace_root: Path::new(".blackpepper/workspaces").to_path_buf(),
        source: CommandSource::Cli,
    };
    let result = run_command("pr", &[String::from("create")], &ctx);
    assert!(!result.ok);
    assert!(result.message.contains("PR generator failed"));
    assert!(result.message.contains("agent_boom"));
}

#[test]
#[cfg(unix)]
fn pr_create_surfaces_gh_failure() {
    let _guard = env_lock();
    let repo = init_repo();
    let config_path = workspace_config_path(repo.path());
    fs::create_dir_all(config_path.parent().expect("config dir")).expect("config dir");

    let command = r#"printf '%b' '<pr>\n  <title>feat(pr): stub create</title>\n  <description>\n## Summary\nStubbed PR description\n  </description>\n</pr>\n' # {{PROMPT}}"#;
    let config = format!("[agent]\ncommand = \"\"\"{command}\"\"\"\n");
    fs::write(&config_path, config).expect("write config");

    let bin_dir = TempDir::new().expect("temp bin");
    let gh_path = bin_dir.path().join("gh");
    let gh_script = "#!/bin/sh\necho \"gh failed\" 1>&2\nexit 1\n";
    fs::write(&gh_path, gh_script).expect("write gh stub");
    let mut perms = fs::metadata(&gh_path).expect("stat gh").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&gh_path, perms).expect("chmod gh");

    let original_path = env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.path().display(), original_path);
    let _path_guard = EnvVarGuard::set("PATH", new_path);

    let ctx = CommandContext {
        cwd: repo.path().to_path_buf(),
        repo_root: Some(repo.path().to_path_buf()),
        workspace_root: Path::new(".blackpepper/workspaces").to_path_buf(),
        source: CommandSource::Cli,
    };
    let result = run_command("pr", &[String::from("create")], &ctx);
    assert!(!result.ok);
    assert!(result.message.contains("github pr create failed"));
    assert!(result.message.contains("gh failed"));
}
