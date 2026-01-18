use std::collections::HashSet;

use crate::commands::{pr, rename};
use crate::config::load_config;
use crate::git::{resolve_repo_root, run_git};
use crate::providers::agent;
use crate::state::rename_workspace_ports;
use crate::tmux;
use crate::workspaces::{
    is_valid_workspace_name, list_workspace_names, workspace_absolute_path,
    workspace_name_from_path,
};

use super::super::pr_command::{format_command_failure, run_shell_with_output};
use super::super::{CommandContext, CommandOutput, CommandPhase, CommandResult, CommandSource};
use super::helpers::{branch_exists, format_exec_output, normalize_workspace_name};

pub(crate) fn workspace_rename<F>(
    args: &[String],
    ctx: &CommandContext,
    on_output: &mut F,
) -> CommandResult
where
    F: FnMut(CommandOutput),
{
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

    let Some(current_name) = workspace_name_from_path(&repo_root, &ctx.workspace_root, &ctx.cwd)
    else {
        return CommandResult {
            ok: false,
            message: "Rename must be run from inside a workspace.".to_string(),
            data: None,
        };
    };

    let used_names: HashSet<String> = list_workspace_names(&repo_root, &ctx.workspace_root)
        .into_iter()
        .collect();
    let raw_name = if args.is_empty() {
        let config = load_config(&repo_root);
        let command_template = if let Some(command) = config.agent.command.as_deref() {
            command
        } else if let Some(provider) = config.agent.provider.as_deref() {
            match agent::provider_command(provider) {
                Some(command) => command,
                None => {
                    return CommandResult {
                        ok: false,
                        message: format!("Unknown agent provider: {provider}."),
                        data: None,
                    }
                }
            }
        } else {
            return CommandResult {
                ok: false,
                message: "Agent provider not configured. Run :rename in the TUI to select one, set agent.provider, or set agent.command in ~/.config/blackpepper/config.toml or .blackpepper/config.toml.".to_string(),
                data: None,
            };
        };

        let script = pr::build_prompt_script(command_template, rename::WORKSPACE_RENAME);
        let mut on_chunk = |chunk: &str| {
            on_output(CommandOutput::Chunk(chunk.to_string()));
        };
        let provider_result = run_shell_with_output(&script, &ctx.cwd, &mut on_chunk);
        on_output(CommandOutput::PhaseComplete(CommandPhase::Agent));
        if !provider_result.ok {
            return CommandResult {
                ok: false,
                message: format_command_failure("Rename generator failed", &provider_result),
                data: None,
            };
        }

        let rename_message = match rename::parse_rename_output(&provider_result.stdout) {
            Ok(message) => message,
            Err(err) => {
                return CommandResult {
                    ok: false,
                    message: err,
                    data: None,
                }
            }
        };
        rename_message.name
    } else {
        args.join(" ")
    };

    let raw_name = raw_name.trim().to_string();
    if raw_name.is_empty() {
        return CommandResult {
            ok: false,
            message: "Workspace name cannot be empty.".to_string(),
            data: None,
        };
    }

    let normalized_name = normalize_workspace_name(&raw_name);
    if normalized_name.is_empty() || !is_valid_workspace_name(&normalized_name) {
        return CommandResult {
            ok: false,
            message: "Workspace name must use lowercase letters, numbers, or dashes.".to_string(),
            data: None,
        };
    }
    if normalized_name == current_name {
        return CommandResult {
            ok: true,
            message: format!("Workspace already named '{current_name}'."),
            data: None,
        };
    }
    if used_names.contains(&normalized_name) {
        return CommandResult {
            ok: false,
            message: format!(
                "Workspace name '{normalized_name}' is already in use. Choose another."
            ),
            data: None,
        };
    }
    if branch_exists(&repo_root, &normalized_name) {
        return CommandResult {
            ok: false,
            message: format!(
                "Branch '{normalized_name}' already exists. Choose another workspace name."
            ),
            data: None,
        };
    }

    let old_path = workspace_absolute_path(&repo_root, &ctx.workspace_root, &current_name);
    if !old_path.is_dir() {
        return CommandResult {
            ok: false,
            message: format!("Workspace '{current_name}' path is missing."),
            data: None,
        };
    }
    let new_path = workspace_absolute_path(&repo_root, &ctx.workspace_root, &normalized_name);
    if new_path.exists() {
        return CommandResult {
            ok: false,
            message: format!("Workspace path '{}' already exists.", new_path.display()),
            data: None,
        };
    }

    let old_path_str = old_path.to_string_lossy().to_string();
    let new_path_str = new_path.to_string_lossy().to_string();
    let move_args = [
        "worktree",
        "move",
        old_path_str.as_str(),
        new_path_str.as_str(),
    ];
    let move_result = run_git(move_args.as_ref(), &repo_root);
    if !move_result.ok {
        let output = format_exec_output(&move_result);
        let details = if output.is_empty() {
            "".to_string()
        } else {
            format!("\n{output}")
        };
        return CommandResult {
            ok: false,
            message: format!("Failed to move workspace '{current_name}'.{details}"),
            data: None,
        };
    }

    let branch_args = ["branch", "-m", normalized_name.as_str()];
    let branch_result = run_git(branch_args.as_ref(), &new_path);
    if !branch_result.ok {
        let output = format_exec_output(&branch_result);
        let details = if output.is_empty() {
            "".to_string()
        } else {
            format!("\n{output}")
        };
        let rollback_result = run_git(
            [
                "worktree",
                "move",
                new_path_str.as_str(),
                old_path_str.as_str(),
            ]
            .as_ref(),
            &repo_root,
        );
        let rollback_output = format_exec_output(&rollback_result);
        let rollback_details = if rollback_result.ok || rollback_output.is_empty() {
            "".to_string()
        } else {
            format!("\nRollback failed: {rollback_output}")
        };
        return CommandResult {
            ok: false,
            message: format!(
                "Failed to rename branch for workspace '{current_name}' to '{normalized_name}'.{details}{rollback_details}"
            ),
            data: None,
        };
    }

    let mut message = format!("Renamed workspace '{current_name}' to '{normalized_name}'.");
    if raw_name != normalized_name {
        message.push_str(&format!(" Normalized from '{raw_name}'."));
    }
    if let Err(err) = rename_workspace_ports(&old_path, &new_path) {
        message.push_str(&format!("\nWarning: failed to move workspace ports: {err}"));
    }

    if ctx.source == CommandSource::Tui {
        let config = load_config(&repo_root);
        let current_session = tmux::session_name(&repo_root, &current_name);
        let next_session = tmux::session_name(&repo_root, &normalized_name);
        if let Err(err) = tmux::rename_session(&config.tmux, &current_session, &next_session) {
            message.push_str(&format!("\nWarning: {err}"));
        }
    }

    CommandResult {
        ok: true,
        message,
        data: Some(normalized_name),
    }
}
