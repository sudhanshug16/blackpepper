//! Git command execution and repository utilities.
//!
//! Provides a thin wrapper around git CLI commands. All git operations
//! go through `run_git` which captures stdout/stderr and exit codes.
//!
//! Key functions:
//! - `run_git`: execute arbitrary git commands
//! - `resolve_repo_root`: find the repository root from any subdirectory

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct ExecResult {
    pub ok: bool,
    #[allow(dead_code)]
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn run_git(args: &[&str], cwd: &Path) -> ExecResult {
    let output = Command::new("git").args(args).current_dir(cwd).output();
    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let exit_code = out.status.code().unwrap_or(-1);
            ExecResult {
                ok: out.status.success(),
                exit_code,
                stdout,
                stderr,
            }
        }
        Err(err) => ExecResult {
            ok: false,
            exit_code: -1,
            stdout: String::new(),
            stderr: err.to_string(),
        },
    }
}

pub fn resolve_repo_root(cwd: &Path) -> Option<PathBuf> {
    let result = run_git(["rev-parse", "--git-common-dir"].as_ref(), cwd);
    if !result.ok {
        return None;
    }

    let git_common = result.stdout.trim();
    if git_common.is_empty() {
        return None;
    }

    let git_path = Path::new(git_common);
    let resolved = if git_path.is_absolute() {
        git_path.to_path_buf()
    } else {
        cwd.join(git_path)
    };

    let canonical = std::fs::canonicalize(&resolved).unwrap_or(resolved);
    canonical.parent().map(|path| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{resolve_repo_root, run_git};
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

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
    fn run_git_reports_success() {
        let repo = init_repo();
        let result = run_git(["rev-parse", "--is-inside-work-tree"].as_ref(), repo.path());
        assert!(result.ok);
        assert_eq!(result.stdout.trim(), "true");
    }

    #[test]
    fn resolve_repo_root_from_subdir() {
        let repo = init_repo();
        let subdir = repo.path().join("nested");
        fs::create_dir_all(&subdir).expect("create subdir");

        let resolved = resolve_repo_root(&subdir).expect("repo root");
        let canonical = fs::canonicalize(repo.path()).unwrap_or_else(|_| repo.path().to_path_buf());
        assert_eq!(resolved, canonical);
    }
}
