//! Workspace path and naming utilities.
//!
//! Workspaces are git worktrees created under a configurable root
//! (default: `.blackpepper/workspaces/<name>`). This module handles:
//! - Path resolution for workspace directories
//! - Name validation (lowercase + dashes only)
//! - Listing existing workspaces via git worktree
//! - Extracting workspace names from paths
//!
//! The actual worktree creation/deletion is in commands/exec.rs.

use std::fs;
use std::path::{Path, PathBuf};

use crate::git::run_git;

pub fn is_valid_workspace_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    for ch in chars {
        if !(ch.is_ascii_lowercase() || ch == '-') {
            return false;
        }
    }
    true
}

pub fn workspace_path(workspace_root: &Path, name: &str) -> PathBuf {
    if name.contains('/') || name.contains('\\') {
        panic!("Workspace name must not include path separators.");
    }
    workspace_root.join(name)
}

pub fn workspace_root_path(repo_root: &Path, workspace_root: &Path) -> PathBuf {
    if workspace_root.is_absolute() {
        workspace_root.to_path_buf()
    } else {
        repo_root.join(workspace_root)
    }
}

pub fn workspace_absolute_path(repo_root: &Path, workspace_root: &Path, name: &str) -> PathBuf {
    workspace_root_path(repo_root, workspace_root).join(name)
}

pub fn ensure_workspace_root(repo_root: &Path, workspace_root: &Path) -> std::io::Result<()> {
    let marker = workspace_root_path(repo_root, workspace_root).join(".pepper-keep");
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(marker, "")
}

pub fn list_workspace_names(repo_root: &Path, workspace_root: &Path) -> Vec<String> {
    let result = run_git(["worktree", "list", "--porcelain"].as_ref(), repo_root);
    if !result.ok {
        return Vec::new();
    }

    let mut names = Vec::new();
    for line in result.stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("worktree ") {
            if let Some(name) = workspace_name_from_path(repo_root, workspace_root, Path::new(rest))
            {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }

    names.sort();
    names
}

pub fn workspace_name_from_path(
    repo_root: &Path,
    workspace_root: &Path,
    worktree_path: &Path,
) -> Option<String> {
    let root = workspace_root_path(repo_root, workspace_root);
    let absolute = if worktree_path.is_absolute() {
        worktree_path.to_path_buf()
    } else {
        repo_root.join(worktree_path)
    };
    let remainder = absolute.strip_prefix(&root).ok()?;
    let name = remainder.iter().next()?.to_string_lossy();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_valid_workspace_name, list_workspace_names, workspace_name_from_path, workspace_path,
        workspace_root_path,
    };
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
        run_git_cmd(&["init", "-b", "main"], repo.path());
        fs::write(repo.path().join("README.md"), "hello").expect("write file");
        run_git_cmd(&["add", "."], repo.path());
        run_git_cmd(&["commit", "-m", "init"], repo.path());
        repo
    }

    #[test]
    fn workspace_name_validation() {
        assert!(is_valid_workspace_name("otter"));
        assert!(is_valid_workspace_name("red-fox"));
        assert!(!is_valid_workspace_name(""));
        assert!(!is_valid_workspace_name("Otter"));
        assert!(!is_valid_workspace_name("red_fox"));
        assert!(!is_valid_workspace_name("-bad"));
    }

    #[test]
    fn workspace_path_rejects_separators() {
        let root = Path::new("/tmp");
        let result = std::panic::catch_unwind(|| workspace_path(root, "a/b"));
        assert!(result.is_err());
    }

    #[test]
    fn workspace_root_path_resolves_relative_and_absolute() {
        let repo = Path::new("/repo");
        let relative = Path::new(".blackpepper/workspaces");
        let absolute = Path::new("/tmp/workspaces");

        assert_eq!(workspace_root_path(repo, relative), repo.join(relative));
        assert_eq!(workspace_root_path(repo, absolute), absolute);
    }

    #[test]
    fn workspace_name_from_paths() {
        let repo = Path::new("/repo");
        let root = Path::new(".blackpepper/workspaces");
        let abs = Path::new("/repo/.blackpepper/workspaces/otter");
        let rel = Path::new(".blackpepper/workspaces/lynx");

        assert_eq!(
            workspace_name_from_path(repo, root, abs),
            Some("otter".to_string())
        );
        assert_eq!(
            workspace_name_from_path(repo, root, rel),
            Some("lynx".to_string())
        );
        assert_eq!(
            workspace_name_from_path(repo, root, Path::new("/repo/other")),
            None
        );
    }

    #[test]
    fn list_workspace_names_reads_worktrees() {
        let repo = init_repo();
        let repo_root = fs::canonicalize(repo.path()).unwrap_or_else(|_| repo.path().to_path_buf());
        let workspace_root = Path::new(".blackpepper/workspaces");
        let root_path = repo_root.join(workspace_root);
        fs::create_dir_all(&root_path).expect("create workspace root");

        let otter_path = root_path.join("otter");
        run_git_cmd(
            &[
                "worktree",
                "add",
                otter_path.to_str().unwrap(),
                "-b",
                "otter",
                "HEAD",
            ],
            &repo_root,
        );

        let names = list_workspace_names(&repo_root, workspace_root);
        assert_eq!(names, vec!["otter".to_string()]);
    }
}
