use crate::config::load_config;
use crate::git::{resolve_repo_root, run_git};
use crate::state::remove_workspace_ports;
use crate::tmux;
use crate::workspaces::{is_valid_workspace_name, workspace_absolute_path, workspace_path};

use super::super::{CommandContext, CommandResult};
use super::helpers::format_exec_output;

/// Destroy a workspace worktree.
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

    let workspace_absolute = workspace_absolute_path(&repo_root, &ctx.workspace_root, name);
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

    let output = format_exec_output(&result);
    let details = if output.is_empty() {
        "".to_string()
    } else {
        format!("\n{output}")
    };
    let port_warning = match remove_workspace_ports(&workspace_absolute) {
        Ok(()) => String::new(),
        Err(err) => format!("\nWarning: failed to remove workspace ports: {err}"),
    };
    CommandResult {
        ok: true,
        message: format!(
            "Removed workspace '{name}' from {}.{details}{port_warning}",
            workspace_path(&ctx.workspace_root, name).to_string_lossy()
        ),
        data: None,
    }
}
