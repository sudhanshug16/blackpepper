use std::fs;
use std::path::{Path, PathBuf};

use crate::git::run_git;

const WORKSPACE_ROOT: &str = "workspaces";

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

pub fn workspace_path(name: &str) -> PathBuf {
    if name.contains('/') || name.contains('\\') {
        panic!("Workspace name must not include path separators.");
    }
    PathBuf::from(WORKSPACE_ROOT).join(name)
}

pub fn workspace_root_path(repo_root: &Path) -> PathBuf {
    repo_root.join(WORKSPACE_ROOT)
}

pub fn workspace_absolute_path(repo_root: &Path, name: &str) -> PathBuf {
    repo_root.join(workspace_path(name))
}

pub fn ensure_workspace_root(repo_root: &Path) -> std::io::Result<()> {
    let marker = workspace_root_path(repo_root).join(".pepper-keep");
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(marker, "")
}

pub fn list_workspace_names(repo_root: &Path) -> Vec<String> {
    let result = run_git(["worktree", "list", "--porcelain"].as_ref(), repo_root);
    if !result.ok {
        return Vec::new();
    }

    let mut names = Vec::new();
    for line in result.stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("worktree ") {
            if let Some(name) = workspace_name_from_path(rest) {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }

    names.sort();
    names
}

pub fn workspace_name_from_path(worktree_path: &str) -> Option<String> {
    let normalized = worktree_path.replace('\\', "/");
    let trimmed = normalized.strip_prefix("./").unwrap_or(&normalized);
    let marker = "/workspaces/";

    if let Some(index) = trimmed.rfind(marker) {
        let remainder = &trimmed[index + marker.len()..];
        let name = remainder.split('/').next().unwrap_or("");
        return if name.is_empty() { None } else { Some(name.to_string()) };
    }

    if let Some(rest) = trimmed.strip_prefix("workspaces/") {
        let name = rest.split('/').next().unwrap_or("");
        return if name.is_empty() { None } else { Some(name.to_string()) };
    }

    None
}
