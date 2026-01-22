//! Command execution handlers.
//!
//! Contains the actual logic for each command. Commands receive
//! a context with current directory and repo information, and
//! return a result with success/failure and a message.

mod init;
mod ports;
mod pr_command;
mod update;
mod workspace;

#[cfg(test)]
mod tests;

use super::registry::{command_help_lines, command_help_lines_cli};
use init::init_project;
use pr_command::{pr_create, pr_sync};
use update::update_command;
use workspace::{
    workspace_create, workspace_destroy, workspace_from_branch, workspace_from_pr, workspace_list,
    workspace_rename, workspace_setup,
};

/// Result of command execution.
#[derive(Debug, Clone)]
pub struct CommandResult {
    pub ok: bool,
    pub message: String,
    /// Optional data returned by command (e.g., created workspace name).
    pub data: Option<String>,
}

/// Context provided to command handlers.
#[derive(Debug, Clone)]
pub struct CommandContext {
    pub cwd: std::path::PathBuf,
    pub repo_root: Option<std::path::PathBuf>,
    pub workspace_root: std::path::PathBuf,
    /// Path to the active workspace (if any).
    pub workspace_path: Option<std::path::PathBuf>,
    pub source: CommandSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSource {
    Tui,
    Cli,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandPhase {
    Agent,
}

#[derive(Debug, Clone)]
pub enum CommandOutput {
    Chunk(String),
    PhaseComplete(CommandPhase),
}

/// Dispatch and execute a command by name.
pub fn run_command(name: &str, args: &[String], ctx: &CommandContext) -> CommandResult {
    let mut noop = |_chunk: CommandOutput| {};
    run_command_with_output(name, args, ctx, &mut noop)
}

/// Dispatch and execute a command by name, streaming output where supported.
pub fn run_command_with_output<F>(
    name: &str,
    args: &[String],
    ctx: &CommandContext,
    on_output: &mut F,
) -> CommandResult
where
    F: FnMut(CommandOutput),
{
    match name {
        "init" => init_project(args, ctx),
        "update" => update_command(),
        "version" => CommandResult {
            ok: true,
            message: format!("blackpepper v{}", env!("CARGO_PKG_VERSION")),
            data: None,
        },
        "help" => CommandResult {
            ok: true,
            message: if ctx.source == CommandSource::Cli {
                command_help_lines_cli().join("\n")
            } else {
                command_help_lines().join("\n")
            },
            data: None,
        },
        "workspace" => {
            let Some(subcommand) = args.first() else {
                return CommandResult {
                    ok: false,
                    message:
                        "Usage: :workspace <list|switch|create|destroy|setup|rename|from-branch|from-pr>"
                            .to_string(),
                    data: None,
                };
            };
            match subcommand.as_str() {
                "create" => workspace_create(&args[1..], ctx),
                "destroy" => workspace_destroy(&args[1..], ctx),
                "setup" => workspace_setup(&args[1..], ctx),
                "rename" => workspace_rename(&args[1..], ctx, on_output),
                "list" => workspace_list(ctx),
                "from-branch" => workspace_from_branch(&args[1..], ctx),
                "from-pr" => workspace_from_pr(&args[1..], ctx),
                "switch" => CommandResult {
                    ok: true,
                    message: "Use :workspace switch <name> to change.".to_string(),
                    data: None,
                },
                _ => CommandResult {
                    ok: false,
                    message:
                        "Usage: :workspace <list|switch|create|destroy|setup|rename|from-branch|from-pr>"
                            .to_string(),
                    data: None,
                },
            }
        }
        "pr" => {
            let Some(subcommand) = args.first() else {
                return CommandResult {
                    ok: false,
                    message: "Usage: :pr <create|sync|open|merge>".to_string(),
                    data: None,
                };
            };
            match subcommand.as_str() {
                "create" => pr_create(ctx, on_output),
                "sync" => pr_sync(ctx, on_output),
                "open" => CommandResult {
                    ok: true,
                    message: "PR opening is not implemented yet.".to_string(),
                    data: None,
                },
                "merge" => CommandResult {
                    ok: true,
                    message: "PR merge is not implemented yet.".to_string(),
                    data: None,
                },
                _ => CommandResult {
                    ok: false,
                    message: "Usage: :pr <create|sync|open|merge>".to_string(),
                    data: None,
                },
            }
        }
        "ports" => ports::ports_list(ctx),
        "quit" | "q" => CommandResult {
            ok: true,
            message: "Exiting.".to_string(),
            data: None,
        },
        _ => CommandResult {
            ok: false,
            message: format!("Unhandled command: {name}"),
            data: None,
        },
    }
}
