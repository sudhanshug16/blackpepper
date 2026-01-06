use std::path::Path;
use std::process::Command;

use crate::commands::pr;
use crate::config::load_config;
use crate::git::{resolve_repo_root, ExecResult};

use super::{CommandContext, CommandResult};

pub(super) fn pr_create(ctx: &CommandContext) -> CommandResult {
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
    let command_template = if let Some(command) = config.agent.command.as_deref() {
        command
    } else if let Some(provider) = config.agent.provider.as_deref() {
        match pr::provider_command(provider) {
            Some(command) => command,
            None => {
                return CommandResult {
                    ok: false,
                    message: format!("Unknown PR provider: {provider}."),
                    data: None,
                }
            }
        }
    } else {
        return CommandResult {
            ok: false,
            message: "Agent provider not configured. Run :pr create in the TUI to select one, set agent.provider, or set agent.command in ~/.config/blackpepper/pepper.toml.".to_string(),
            data: None,
        };
    };

    let script = pr::build_prompt_script(command_template, pr::PR_CREATE);
    let provider_result = run_shell(&script, &ctx.cwd);
    if !provider_result.ok {
        return CommandResult {
            ok: false,
            message: format_command_failure("PR generator failed", &provider_result),
            data: None,
        };
    }

    let pr_message = match pr::parse_pr_output(&provider_result.stdout) {
        Ok(message) => message,
        Err(err) => {
            return CommandResult {
                ok: false,
                message: err,
                data: None,
            }
        }
    };

    let gh_result = run_gh_create(&ctx.cwd, &pr_message.title, &pr_message.description);
    if !gh_result.ok {
        return CommandResult {
            ok: false,
            message: format_command_failure("gh pr create failed", &gh_result),
            data: None,
        };
    }

    let output = gh_result.stdout.trim();
    CommandResult {
        ok: true,
        message: if output.is_empty() {
            "Pull request created.".to_string()
        } else {
            output.to_string()
        },
        data: None,
    }
}

fn run_shell(script: &str, cwd: &Path) -> ExecResult {
    let output = Command::new("sh")
        .arg("-c")
        .arg(script)
        .current_dir(cwd)
        .output();
    match output {
        Ok(out) => ExecResult {
            ok: out.status.success(),
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        },
        Err(err) => ExecResult {
            ok: false,
            exit_code: -1,
            stdout: String::new(),
            stderr: err.to_string(),
        },
    }
}

fn run_gh_create(cwd: &Path, title: &str, body: &str) -> ExecResult {
    let output = Command::new("gh")
        .args(["pr", "create", "--title"])
        .arg(title)
        .args(["--body"])
        .arg(body)
        .current_dir(cwd)
        .output();
    match output {
        Ok(out) => ExecResult {
            ok: out.status.success(),
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        },
        Err(err) => ExecResult {
            ok: false,
            exit_code: -1,
            stdout: String::new(),
            stderr: err.to_string(),
        },
    }
}

fn format_command_failure(prefix: &str, result: &ExecResult) -> String {
    let detail = if !result.stderr.trim().is_empty() {
        result.stderr.trim()
    } else {
        result.stdout.trim()
    };
    if detail.is_empty() {
        format!("{prefix}.")
    } else {
        format!("{prefix}: {detail}")
    }
}
