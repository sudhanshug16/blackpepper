use std::collections::HashSet;

use crate::git::{resolve_repo_root, run_git};
use crate::workspaces::{
    ensure_workspace_root, is_valid_workspace_name, list_workspace_names, workspace_path,
};

use super::super::{CommandContext, CommandResult};
use super::helpers::{branch_exists, format_exec_output, pick_unused_animal_name};

/// Create a new workspace worktree.
pub(crate) fn workspace_create(args: &[String], ctx: &CommandContext) -> CommandResult {
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

    let used_names: HashSet<String> = list_workspace_names(&repo_root, &ctx.workspace_root)
        .into_iter()
        .collect();
    let mut workspace_name = args.first().cloned();
    if workspace_name.is_none() {
        workspace_name = pick_unused_animal_name(&used_names);
    }
    let Some(workspace_name) = workspace_name else {
        return CommandResult {
            ok: false,
            message: "No unused animal names available. Use :workspace create <unique-name>."
                .to_string(),
            data: None,
        };
    };

    if !is_valid_workspace_name(&workspace_name) {
        return CommandResult {
            ok: false,
            message: "Workspace name must use lowercase letters, numbers, or dashes.".to_string(),
            data: None,
        };
    }
    if used_names.contains(&workspace_name) {
        return CommandResult {
            ok: false,
            message: format!(
                "Workspace name '{workspace_name}' is already in use. Choose another."
            ),
            data: None,
        };
    }
    if branch_exists(&repo_root, &workspace_name) {
        return CommandResult {
            ok: false,
            message: format!(
                "Branch '{workspace_name}' already exists. Choose another workspace name."
            ),
            data: None,
        };
    }

    let worktree_path = workspace_path(&ctx.workspace_root, &workspace_name);
    let worktree_path_str = worktree_path.to_string_lossy().to_string();
    let git_args = [
        "worktree",
        "add",
        worktree_path_str.as_str(),
        "-b",
        workspace_name.as_str(),
        "HEAD",
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
    CommandResult {
        ok: true,
        message: format!(
            "Created workspace '{workspace_name}' at {}.{details}",
            worktree_path.to_string_lossy()
        ),
        data: Some(workspace_name),
    }
}
