use crate::config::load_config;
use crate::git::{resolve_repo_root, run_git};
use crate::tmux;
use crate::workspaces::{
    ensure_workspace_root, is_valid_workspace_name, list_workspace_names, workspace_absolute_path,
    workspace_name_from_path, workspace_path,
};
use std::collections::HashSet;

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
    let message = format!(
        "Created workspace '{workspace_name}' at {}.{details}",
        worktree_path.to_string_lossy()
    );
    CommandResult {
        ok: true,
        message,
        data: Some(workspace_name),
    }
}

pub(crate) fn workspace_setup(args: &[String], ctx: &CommandContext) -> CommandResult {
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

    let workspace_name = if let Some(name) = args.first() {
        name.to_string()
    } else {
        match workspace_name_from_path(&repo_root, &ctx.workspace_root, &ctx.cwd) {
            Some(name) => name,
            None => {
                return CommandResult {
                    ok: false,
                    message: "Provide a workspace name or run this from a workspace.".to_string(),
                    data: None,
                };
            }
        }
    };

    if !is_valid_workspace_name(&workspace_name) {
        return CommandResult {
            ok: false,
            message: "Workspace name must use lowercase letters, numbers, or dashes.".to_string(),
            data: None,
        };
    }

    let workspace_path = workspace_absolute_path(&repo_root, &ctx.workspace_root, &workspace_name);
    if !workspace_path.exists() {
        return CommandResult {
            ok: false,
            message: format!("Workspace '{workspace_name}' does not exist."),
            data: None,
        };
    }

    let config = load_config(&repo_root);
    let setup_body = match tmux::setup_shell_body(&config.workspace.setup_scripts) {
        Some(body) => body,
        None => {
            return CommandResult {
                ok: true,
                message: format!("No workspace setup scripts configured for '{workspace_name}'."),
                data: None,
            };
        }
    };
    let setup_command = match tmux::setup_command_args(&config.workspace.setup_scripts) {
        Some(command) => command,
        None => {
            return CommandResult {
                ok: false,
                message: format!("Setup scripts are invalid for '{workspace_name}'."),
                data: None,
            };
        }
    };

    let session_name = tmux::session_name(&repo_root, &workspace_name);
    let tabs = tmux::resolve_tabs(&config.tmux);
    let setup_tab = tmux::SetupTab {
        name: tmux::SETUP_TMUX_TAB.to_string(),
        command: setup_command,
    };
    let created = match tmux::ensure_session_layout(
        &config.tmux,
        &session_name,
        &workspace_path,
        Some(setup_tab),
        &tabs,
    ) {
        Ok(created) => created,
        Err(err) => {
            return CommandResult {
                ok: false,
                message: format!("Failed to prepare tmux session for '{workspace_name}': {err}"),
                data: None,
            };
        }
    };

    if created {
        return CommandResult {
            ok: true,
            message: format!(
                "Started tmux session '{session_name}' and running setup scripts in '{}'.",
                tmux::SETUP_TMUX_TAB
            ),
            data: None,
        };
    }

    let target = format!("{session_name}:0");
    let _ = tmux::rename_window(&config.tmux, &target, tmux::SETUP_TMUX_TAB);
    if let Err(err) = tmux::send_keys(&config.tmux, &target, &setup_body) {
        return CommandResult {
            ok: false,
            message: format!("Workspace setup failed for '{workspace_name}': {err}"),
            data: None,
        };
    }

    CommandResult {
        ok: true,
        message: format!(
            "Running setup scripts for '{workspace_name}' in tmux tab '{}'.",
            tmux::SETUP_TMUX_TAB
        ),
        data: None,
    }
}
