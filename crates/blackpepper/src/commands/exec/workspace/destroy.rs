use crate::config::load_config;
use crate::git::{resolve_repo_root, run_git};
use crate::tmux;
use crate::workspaces::{is_valid_workspace_name, workspace_path};

use super::super::{CommandContext, CommandResult};
use super::helpers::{branch_exists, format_exec_output};

/// Destroy a workspace worktree and its branch.
pub(crate) fn workspace_destroy(args: &[String], ctx: &CommandContext) -> CommandResult {
    let Some(name) = args.first() else {
        return CommandResult {
            ok: false,
            message: "Usage: :workspace destroy <animal>".to_string(),
            data: None,
        };
    };
    if !is_valid_workspace_name(name) {
        return CommandResult {
            ok: false,
            message: "Workspace name must use lowercase letters, numbers, or dashes.".to_string(),
            data: None,
        };
    }
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

    let config = load_config(&repo_root);
    let session_name = tmux::session_name(&repo_root, name);
    if let Err(err) = tmux::kill_session(&config.tmux, &session_name) {
        return CommandResult {
            ok: false,
            message: format!("Failed to kill tmux session '{session_name}': {err}"),
            data: None,
        };
    }

    let worktree_path_str = workspace_path(&ctx.workspace_root, name)
        .to_string_lossy()
        .to_string();
    let git_args = ["worktree", "remove", worktree_path_str.as_str()];
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
            message: format!("Failed to remove workspace '{name}'.{details}"),
            data: None,
        };
    }

    let mut deleted_branch = false;
    if branch_exists(&repo_root, name) {
        let git_args = ["branch", "-D", name.as_str()];
        let branch_result = run_git(git_args.as_ref(), &repo_root);
        if !branch_result.ok {
            let output = format_exec_output(&branch_result);
            let details = if output.is_empty() {
                "".to_string()
            } else {
                format!("\n{output}")
            };
            return CommandResult {
                ok: false,
                message: format!(
                    "Removed workspace '{name}', but failed to delete branch '{name}'.{details}"
                ),
                data: None,
            };
        }
        deleted_branch = true;
    }

    let output = format_exec_output(&result);
    let details = if output.is_empty() {
        "".to_string()
    } else {
        format!("\n{output}")
    };
    CommandResult {
        ok: true,
        message: if deleted_branch {
            format!(
                "Removed workspace '{name}' from {} and deleted its branch.{details}",
                workspace_path(&ctx.workspace_root, name).to_string_lossy()
            )
        } else {
            format!(
                "Removed workspace '{name}' from {}.{details}",
                workspace_path(&ctx.workspace_root, name).to_string_lossy()
            )
        },
        data: None,
    }
}
