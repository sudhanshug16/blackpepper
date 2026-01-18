//! Create workspace from an existing remote branch.

use std::collections::HashSet;

use crate::config::load_config;
use crate::git::{resolve_repo_root, run_git};
use crate::workspaces::{
    ensure_workspace_root, is_valid_workspace_name, list_workspace_names, workspace_path,
};

use super::super::{CommandContext, CommandResult};
use super::helpers::{branch_exists, format_exec_output, normalize_workspace_name};

/// Create a workspace from an existing remote branch.
///
/// Fetches the branch from the configured remote and creates a worktree
/// tracking it. The workspace name is derived from the branch name,
/// normalized to valid workspace characters.
pub(crate) fn workspace_from_branch(args: &[String], ctx: &CommandContext) -> CommandResult {
    let Some(branch_name) = args.first() else {
        return CommandResult {
            ok: false,
            message: "Usage: :workspace from-branch <branch>".to_string(),
            data: None,
        };
    };

    let repo_root = ctx
        .repo_root
        .clone()
        .or_else(|| resolve_repo_root(&ctx.cwd))
        .ok_or_else(|| CommandResult {
            ok: false,
            message: "Not inside a git repository.".to_string(),
            data: None,
        });
    let repo_root = match repo_root {
        Ok(root) => root,
        Err(result) => return result,
    };

    if let Err(error) = ensure_workspace_root(&repo_root, &ctx.workspace_root) {
        return CommandResult {
            ok: false,
            message: format!("Failed to create workspace root: {error}"),
            data: None,
        };
    }

    let config = load_config(&repo_root);
    let remote = config.git.remote.trim();
    let remote = if remote.is_empty() { "origin" } else { remote };

    // Normalize branch name to valid workspace name
    let workspace_name = normalize_workspace_name(branch_name);
    if workspace_name.is_empty() || !is_valid_workspace_name(&workspace_name) {
        return CommandResult {
            ok: false,
            message: format!(
                "Branch name '{branch_name}' cannot be normalized to a valid workspace name. \
                 Workspace names must use lowercase letters, numbers, or dashes."
            ),
            data: None,
        };
    }

    // Check if workspace name is already in use
    let used_names: HashSet<String> = list_workspace_names(&repo_root, &ctx.workspace_root)
        .into_iter()
        .collect();
    if used_names.contains(&workspace_name) {
        return CommandResult {
            ok: false,
            message: format!(
                "Workspace name '{workspace_name}' is already in use. Choose another."
            ),
            data: None,
        };
    }

    // Check if local branch already exists
    if branch_exists(&repo_root, &workspace_name) {
        return CommandResult {
            ok: false,
            message: format!(
                "Local branch '{workspace_name}' already exists. \
                 Remove it first or use a different branch."
            ),
            data: None,
        };
    }

    // Fetch the remote branch
    let fetch_args = ["fetch", remote, branch_name];
    let fetch_result = run_git(fetch_args.as_ref(), &repo_root);
    if !fetch_result.ok {
        let output = format_exec_output(&fetch_result);
        let details = if output.is_empty() {
            "".to_string()
        } else {
            format!("\n{output}")
        };
        return CommandResult {
            ok: false,
            message: format!("Failed to fetch branch '{branch_name}' from '{remote}'.{details}"),
            data: None,
        };
    }

    // Create worktree tracking the remote branch
    let worktree_path = workspace_path(&ctx.workspace_root, &workspace_name);
    let worktree_path_str = worktree_path.to_string_lossy().to_string();
    let remote_ref = format!("{remote}/{branch_name}");
    let git_args = [
        "worktree",
        "add",
        worktree_path_str.as_str(),
        "-b",
        workspace_name.as_str(),
        remote_ref.as_str(),
    ];
    let result = run_git(git_args.as_ref(), &repo_root);
    if !result.ok {
        let output = format_exec_output(&result);
        let details = if output.is_empty() {
            "".to_string()
        } else {
            format!("\n{output}")
        };
        return CommandResult {
            ok: false,
            message: format!("Failed to create workspace '{workspace_name}'.{details}"),
            data: None,
        };
    }

    let output = format_exec_output(&result);
    let details = if output.is_empty() {
        "".to_string()
    } else {
        format!("\n{output}")
    };
    let normalized_note = if workspace_name != *branch_name {
        format!(" (normalized from '{branch_name}')")
    } else {
        String::new()
    };
    let message = format!(
        "Created workspace '{workspace_name}'{normalized_note} at {}.{details}",
        worktree_path.to_string_lossy()
    );
    CommandResult {
        ok: true,
        message,
        data: Some(workspace_name),
    }
}
