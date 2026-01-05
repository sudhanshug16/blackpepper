use std::fs;
use std::path::{Path, PathBuf};

use crate::git::run_git;

pub fn is_valid_workspace_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else { return false };
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
            if let Some(name) = workspace_name_from_path(repo_root, workspace_root, Path::new(rest)) {
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
