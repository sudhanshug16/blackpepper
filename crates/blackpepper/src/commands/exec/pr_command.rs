use std::env;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::commands::pr;
use crate::config::{load_config, Config, TmuxConfig};
use crate::git::{resolve_repo_root, run_git, ExecResult};
use crate::providers::{agent, upstream};
use crate::tmux;
use crate::workspaces::workspace_name_from_path;

use super::{CommandContext, CommandOutput, CommandResult, CommandSource};

pub(super) fn pr_create<F>(ctx: &CommandContext, on_output: &mut F) -> CommandResult
where
    F: FnMut(CommandOutput),
{
    let repo_root = match resolve_repo_root_for_command(ctx) {
        Ok(root) => root,
        Err(result) => return result,
    };

    let config = load_config(&repo_root);
    if ctx.source == CommandSource::Tui {
        let command_args = vec!["pr".to_string(), "create".to_string()];
        return spawn_cli_command_in_tmux(ctx, &repo_root, &config.tmux, &command_args);
    }

    let provider_result = match run_agent_prompt(ctx, &config, pr::PR_CREATE) {
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
    if ctx.source == CommandSource::Tui {
        let command_args = vec!["pr".to_string(), "sync".to_string()];
        return spawn_cli_command_in_tmux(ctx, &repo_root, &config.tmux, &command_args);
    }
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

    let provider_result = match run_agent_prompt(ctx, &config, pr::COMMIT_CHANGES) {
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

pub(crate) fn run_agent_prompt(
    ctx: &CommandContext,
    config: &Config,
    prompt: &str,
) -> Result<ExecResult, CommandResult> {
    let command_template = resolve_agent_command(config)?;
    let script = pr::build_prompt_script(command_template, prompt);
    let cwd = ctx.workspace_path.as_ref().unwrap_or(&ctx.cwd);
    Ok(run_shell_with_output(&script, cwd))
}

pub(crate) fn spawn_cli_command_in_tmux(
    ctx: &CommandContext,
    repo_root: &Path,
    tmux_config: &TmuxConfig,
    args: &[String],
) -> CommandResult {
    let workspace_path = ctx.workspace_path.as_ref().unwrap_or(&ctx.cwd);
    let workspace_name =
        match workspace_name_from_path(repo_root, &ctx.workspace_root, workspace_path) {
            Some(name) => name,
            None => {
                return CommandResult {
                    ok: false,
                    message: "Unable to resolve workspace name for tmux session.".to_string(),
                    data: None,
                }
            }
        };
    let session = tmux::session_name(repo_root, &workspace_name);
    let target = match tmux::ensure_window(
        tmux_config,
        &session,
        tmux::PR_AGENT_TMUX_TAB,
        workspace_path,
    ) {
        Ok(target) => target,
        Err(err) => {
            return CommandResult {
                ok: false,
                message: err,
                data: None,
            }
        }
    };
    let exe_path = match env::current_exe() {
        Ok(path) => path,
        Err(err) => {
            return CommandResult {
                ok: false,
                message: format!("Failed to resolve bp executable: {err}"),
                data: None,
            }
        }
    };
    let command_line = build_shell_command(&exe_path, args);
    let command = tmux_command_args(&command_line);
    if let Err(err) = tmux::respawn_pane(tmux_config, &target, workspace_path, &command) {
        return CommandResult {
            ok: false,
            message: err,
            data: None,
        };
    }
    CommandResult {
        ok: true,
        message: format!("Started command in tmux tab '{}'.", tmux::PR_AGENT_TMUX_TAB),
        data: None,
    }
}

#[cfg(not(windows))]
fn build_shell_command(exe: &Path, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(shell_quote(&exe.to_string_lossy()));
    for arg in args {
        parts.push(shell_quote(arg));
    }
    parts.join(" ")
}

#[cfg(windows)]
fn build_shell_command(exe: &Path, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(format!("\"{}\"", exe.to_string_lossy()));
    for arg in args {
        parts.push(format!("\"{}\"", arg.replace('"', "\"\"")));
    }
    parts.join(" ")
}

#[cfg(not(windows))]
fn shell_quote(value: &str) -> String {
    let escaped = value.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

#[cfg(not(windows))]
fn tmux_command_args(command: &str) -> Vec<String> {
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    vec![
        shell,
        "-lic".to_string(),
        format!("{command}; exec \"${{SHELL:-/bin/sh}}\""),
    ]
}

#[cfg(windows)]
fn tmux_command_args(command: &str) -> Vec<String> {
    vec!["cmd".to_string(), "/K".to_string(), command.to_string()]
}

fn run_shell_with_output(script: &str, cwd: &Path) -> ExecResult {
    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(script)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return ExecResult {
                ok: false,
                exit_code: -1,
                stdout: String::new(),
                stderr: err.to_string(),
            }
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_handle = stdout.map(|stream| spawn_reader(stream, true));
    let stderr_handle = stderr.map(|stream| spawn_reader(stream, false));

    let stdout_output = stdout_handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let stderr_output = stderr_handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();

    let status = match child.wait() {
        Ok(status) => status,
        Err(err) => {
            return ExecResult {
                ok: false,
                exit_code: -1,
                stdout: stdout_output,
                stderr: err.to_string(),
            }
        }
    };

    ExecResult {
        ok: status.success(),
        exit_code: status.code().unwrap_or(-1),
        stdout: stdout_output,
        stderr: stderr_output,
    }
}

fn spawn_reader<R: std::io::Read + Send + 'static>(
    reader: R,
    is_stdout: bool,
) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut output = String::new();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            let bytes = match reader.read_line(&mut line) {
                Ok(bytes) => bytes,
                Err(_) => break,
            };
            if bytes == 0 {
                break;
            }
            output.push_str(&line);
            if is_stdout {
                print!("{line}");
                let _ = std::io::stdout().flush();
            } else {
                eprint!("{line}");
                let _ = std::io::stderr().flush();
            }
        }
        output
    })
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
