use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tempfile::TempDir;

use crate::commands::pr;
use crate::config::{load_config, Config, TmuxConfig};
use crate::git::{resolve_repo_root, run_git, ExecResult};
use crate::providers::{agent, upstream};
use crate::tmux;
use crate::workspaces::workspace_name_from_path;

use super::{CommandContext, CommandOutput, CommandPhase, CommandResult};

pub(super) fn pr_create<F>(ctx: &CommandContext, on_output: &mut F) -> CommandResult
where
    F: FnMut(CommandOutput),
{
    let repo_root = match resolve_repo_root_for_command(ctx) {
        Ok(root) => root,
        Err(result) => return result,
    };

    let config = load_config(&repo_root);
    let provider_result = match run_agent_prompt(ctx, &repo_root, &config, pr::PR_CREATE, on_output)
    {
        Ok(result) => result,
        Err(result) => return result,
    };
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

    let upstream_provider = config.upstream.provider.trim();
    let upstream_provider = if upstream_provider.is_empty() {
        upstream::DEFAULT_PROVIDER
    } else {
        upstream_provider
    };
    let gh_result = match upstream::create_pr(
        upstream_provider,
        &ctx.cwd,
        &pr_message.title,
        &pr_message.description,
    ) {
        Ok(result) => result,
        Err(err) => {
            return CommandResult {
                ok: false,
                message: err,
                data: None,
            }
        }
    };
    append_upstream_output(on_output, &gh_result);
    if !gh_result.ok {
        return CommandResult {
            ok: false,
            message: format_command_failure("github pr create failed", &gh_result),
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

pub(super) fn pr_sync<F>(ctx: &CommandContext, on_output: &mut F) -> CommandResult
where
    F: FnMut(CommandOutput),
{
    let repo_root = match resolve_repo_root_for_command(ctx) {
        Ok(root) => root,
        Err(result) => return result,
    };
    let config = load_config(&repo_root);
    let cwd = ctx.workspace_path.as_ref().unwrap_or(&ctx.cwd);

    let pull_result = run_git(&["pull", "--rebase", "--autostash"], cwd);
    append_upstream_output(on_output, &pull_result);
    if !pull_result.ok {
        return CommandResult {
            ok: false,
            message: format_command_failure("git pull --rebase failed", &pull_result),
            data: None,
        };
    }

    let provider_result =
        match run_agent_prompt(ctx, &repo_root, &config, pr::COMMIT_CHANGES, on_output) {
            Ok(result) => result,
            Err(result) => return result,
        };
    if !provider_result.ok {
        return CommandResult {
            ok: false,
            message: format_command_failure("Commit generator failed", &provider_result),
            data: None,
        };
    }

    if let Err(err) = pr::parse_commit_output(&provider_result.stdout) {
        return CommandResult {
            ok: false,
            message: err,
            data: None,
        };
    }

    CommandResult {
        ok: true,
        message: "PR sync complete.".to_string(),
        data: None,
    }
}

fn resolve_repo_root_for_command(ctx: &CommandContext) -> Result<PathBuf, CommandResult> {
    ctx.repo_root
        .clone()
        .or_else(|| resolve_repo_root(&ctx.cwd))
        .ok_or_else(|| CommandResult {
            ok: false,
            message: "Not inside a git repository.".to_string(),
            data: None,
        })
}

fn resolve_agent_command<'a>(config: &'a Config) -> Result<&'a str, CommandResult> {
    if let Some(command) = config.agent.command.as_deref() {
        return Ok(command);
    }

    if let Some(provider) = config.agent.provider.as_deref() {
        return agent::provider_command(provider).ok_or_else(|| CommandResult {
            ok: false,
            message: format!("Unknown agent provider: {provider}."),
            data: None,
        });
    }

    Err(CommandResult {
        ok: false,
        message: "Agent provider not configured. Set agent.provider or agent.command in ~/.config/blackpepper/config.toml or .blackpepper/config.toml.".to_string(),
        data: None,
    })
}

pub(crate) fn run_agent_prompt<F>(
    ctx: &CommandContext,
    repo_root: &Path,
    config: &Config,
    prompt: &str,
    on_output: &mut F,
) -> Result<ExecResult, CommandResult>
where
    F: FnMut(CommandOutput),
{
    let command_template = resolve_agent_command(config)?;
    let script = pr::build_prompt_script(command_template, prompt);
    on_output(CommandOutput::Chunk(format!(
        "Running agent in tmux tab '{}'.",
        tmux::PR_AGENT_TMUX_TAB
    )));
    let provider_result = match run_agent_in_tmux(ctx, repo_root, &config.tmux, &script) {
        Ok(result) => result,
        Err(err) => {
            return Err(CommandResult {
                ok: false,
                message: err,
                data: None,
            })
        }
    };
    on_output(CommandOutput::PhaseComplete(CommandPhase::Agent));
    Ok(provider_result)
}

fn run_agent_in_tmux(
    ctx: &CommandContext,
    repo_root: &Path,
    tmux_config: &TmuxConfig,
    script: &str,
) -> Result<ExecResult, String> {
    let workspace_path = ctx.workspace_path.as_ref().unwrap_or(&ctx.cwd);
    let workspace_name =
        workspace_name_from_path(repo_root, &ctx.workspace_root, workspace_path)
            .ok_or_else(|| "Unable to resolve workspace name for tmux session.".to_string())?;
    let session = tmux::session_name(repo_root, &workspace_name);
    let target = tmux::ensure_window(
        tmux_config,
        &session,
        tmux::PR_AGENT_TMUX_TAB,
        workspace_path,
    )?;

    let temp_dir =
        TempDir::new().map_err(|err| format!("Failed to create agent temp dir: {err}"))?;
    let prompt_path = temp_dir.path().join("agent-prompt.sh");
    fs::write(&prompt_path, script)
        .map_err(|err| format!("Failed to write agent prompt script: {err}"))?;
    let output_path = temp_dir.path().join("agent-output.txt");
    let status_path = temp_dir.path().join("agent-status.txt");
    let runner_path = temp_dir.path().join("agent-runner.sh");
    let runner_script = r#"#!/usr/bin/env bash
set -o pipefail
script_path="$1"
output_path="$2"
status_path="$3"
bash "$script_path" 2>&1 | tee "$output_path"
status=$?
printf "%s\n" "$status" > "$status_path"
"#;
    fs::write(&runner_path, runner_script)
        .map_err(|err| format!("Failed to write agent runner script: {err}"))?;

    let command = vec![
        "bash".to_string(),
        runner_path.to_string_lossy().to_string(),
        prompt_path.to_string_lossy().to_string(),
        output_path.to_string_lossy().to_string(),
        status_path.to_string_lossy().to_string(),
    ];
    tmux::respawn_pane(tmux_config, &target, workspace_path, &command)
        .map_err(|err| format!("Failed to launch agent in tmux: {err}"))?;

    let exit_code = wait_for_status(&status_path, Duration::from_secs(60 * 30))?;
    let stdout = fs::read_to_string(&output_path)
        .map_err(|err| format!("Failed to read agent output: {err}"))?;

    Ok(ExecResult {
        ok: exit_code == 0,
        exit_code,
        stdout,
        stderr: String::new(),
    })
}

fn wait_for_status(path: &Path, timeout: Duration) -> Result<i32, String> {
    let start = Instant::now();
    loop {
        if path.exists() {
            let content = fs::read_to_string(path)
                .map_err(|err| format!("Failed to read agent status: {err}"))?;
            let trimmed = content.trim();
            if trimmed.is_empty() {
                return Err("Agent status file was empty.".to_string());
            }
            return trimmed
                .parse::<i32>()
                .map_err(|err| format!("Failed to parse agent status: {err}"));
        }
        if start.elapsed() > timeout {
            return Err("Timed out waiting for agent output.".to_string());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn append_upstream_output<F>(on_output: &mut F, result: &ExecResult)
where
    F: FnMut(CommandOutput),
{
    let stdout = result.stdout.trim();
    let stderr = result.stderr.trim();
    if stdout.is_empty() && stderr.is_empty() {
        return;
    }
    let mut chunk = String::new();
    if !stdout.is_empty() {
        chunk.push_str(stdout);
    }
    if !stderr.is_empty() {
        if !chunk.is_empty() {
            chunk.push('\n');
        }
        chunk.push_str(stderr);
    }
    on_output(CommandOutput::Chunk(format!("\n{chunk}\n")));
}

pub(super) fn format_command_failure(prefix: &str, result: &ExecResult) -> String {
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
