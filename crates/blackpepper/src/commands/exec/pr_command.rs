use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Sender};

use crate::commands::pr;
use crate::config::load_config;
use crate::git::{resolve_repo_root, ExecResult};
use crate::providers::{agent, upstream};

use super::{CommandContext, CommandOutput, CommandPhase, CommandResult};

pub(super) fn pr_create<F>(ctx: &CommandContext, on_output: &mut F) -> CommandResult
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
            message: "Agent provider not configured. Run :pr create in the TUI to select one, set agent.provider, or set agent.command in ~/.config/blackpepper/pepper.toml.".to_string(),
            data: None,
        };
    };

    let script = pr::build_prompt_script(command_template, pr::PR_CREATE);
    let mut on_chunk = |chunk: &str| {
        on_output(CommandOutput::Chunk(chunk.to_string()));
    };
    let provider_result = run_shell_with_output(&script, &ctx.cwd, &mut on_chunk);
    on_output(CommandOutput::PhaseComplete(CommandPhase::Agent));
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

fn run_shell_with_output<F>(script: &str, cwd: &Path, on_output: &mut F) -> ExecResult
where
    F: FnMut(&str),
{
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
    let (tx, rx) = mpsc::channel();

    let stdout_handle = stdout.map(|stream| spawn_reader(stream, tx.clone()));
    let stderr_handle = stderr.map(|stream| spawn_reader(stream, tx.clone()));
    drop(tx);

    for chunk in rx {
        on_output(&chunk);
    }

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
    tx: Sender<String>,
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
            let _ = tx.send(line.clone());
        }
        output
    })
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
