use std::path::Path;
use std::process::{Command, Output};

use crate::config::TmuxConfig;

const DEFAULT_TMUX_COMMAND: &str = "tmux";

pub fn session_name(repo_root: &Path, workspace: &str) -> String {
    let repo_dir = repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repo");
    let repo = sanitize_component(repo_dir, "repo");
    let workspace = sanitize_component(workspace, "workspace");
    format!("{repo}:{workspace}")
}

pub fn client_command(
    config: &TmuxConfig,
    session: &str,
    cwd: &Path,
) -> (String, Vec<String>) {
    let command = resolve_command(config);
    let mut args = config.args.clone();
    args.extend([
        "new-session".to_string(),
        "-A".to_string(),
        "-s".to_string(),
        session.to_string(),
        "-c".to_string(),
        cwd.to_string_lossy().to_string(),
    ]);
    (command, args)
}

pub fn kill_session(config: &TmuxConfig, session: &str) -> Result<bool, String> {
    if !has_session(config, session)? {
        return Ok(false);
    }
    let output = run_tmux(config, &["kill-session", "-t", session])?;
    if output.status.success() {
        Ok(true)
    } else {
        Err(format!(
            "Failed to kill tmux session '{session}'.{}",
            format_output(&output)
        ))
    }
}

fn has_session(config: &TmuxConfig, session: &str) -> Result<bool, String> {
    let output = run_tmux(config, &["has-session", "-t", session])?;
    Ok(output.status.success())
}

fn run_tmux(config: &TmuxConfig, args: &[&str]) -> Result<Output, String> {
    let mut cmd = Command::new(resolve_command(config));
    cmd.args(&config.args);
    cmd.args(args);
    cmd.output().map_err(|err| err.to_string())
}

fn resolve_command(config: &TmuxConfig) -> String {
    config
        .command
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_TMUX_COMMAND)
        .to_string()
}

fn sanitize_component(value: &str, fallback: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn format_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut parts = Vec::new();
    if !stdout.trim().is_empty() {
        parts.push(stdout.trim().to_string());
    }
    if !stderr.trim().is_empty() {
        parts.push(stderr.trim().to_string());
    }
    if parts.is_empty() {
        "".to_string()
    } else {
        format!(" {}", parts.join("\n"))
    }
}
