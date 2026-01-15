use std::env;
use std::path::Path;
use std::process::{Command, Output};

pub const DEFAULT_TMUX_TAB: &str = "work";
pub const SETUP_TMUX_TAB: &str = "setup";

use crate::config::{TmuxConfig, TmuxTabConfig};

const DEFAULT_TMUX_COMMAND: &str = "tmux";

#[derive(Debug, Clone)]
pub struct SetupTab {
    pub name: String,
    pub command: Vec<String>,
}

pub fn session_name(repo_root: &Path, workspace: &str) -> String {
    let repo_dir = repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repo");
    let repo = sanitize_component(repo_dir, "repo");
    let workspace = sanitize_component(workspace, "workspace");
    format!("{repo}:{workspace}")
}

pub fn client_command(config: &TmuxConfig, session: &str, cwd: &Path) -> (String, Vec<String>) {
    let command = resolve_command(config);
    let mut args = config.args.clone();
    args.extend([
        "new-session".to_string(),
        "-A".to_string(),
        "-s".to_string(),
        session.to_string(),
        "-c".to_string(),
        cwd.to_string_lossy().to_string(),
        ";".to_string(),
        "set-option".to_string(),
        "-gq".to_string(),
        "extended-keys".to_string(),
        "on".to_string(),
    ]);
    if let Some(term) = truecolor_term() {
        args.extend([
            ";".to_string(),
            "set-option".to_string(),
            "-gaq".to_string(),
            "terminal-overrides".to_string(),
            format!(",{term}:Tc"),
        ]);
    }
    (command, args)
}

pub fn ensure_session_layout(
    config: &TmuxConfig,
    session: &str,
    cwd: &Path,
    setup: Option<SetupTab>,
    tabs: &[TmuxTabConfig],
) -> Result<bool, String> {
    if has_session(config, session)? {
        return Ok(false);
    }

    match setup {
        Some(setup_tab) => {
            new_session(
                config,
                session,
                cwd,
                Some(&setup_tab.name),
                Some(&setup_tab.command),
            )?;
            for tab in tabs {
                let command = tab_command_args(tab.command.as_deref());
                new_window(config, session, &tab.name, cwd, command.as_deref())?;
            }
        }
        None => {
            let (first, rest) = tabs
                .split_first()
                .ok_or_else(|| "No tmux tabs configured.".to_string())?;
            let command = tab_command_args(first.command.as_deref());
            new_session(config, session, cwd, Some(&first.name), command.as_deref())?;
            for tab in rest {
                let command = tab_command_args(tab.command.as_deref());
                new_window(config, session, &tab.name, cwd, command.as_deref())?;
            }
        }
    }

    Ok(true)
}

pub fn resolve_tabs(config: &TmuxConfig) -> Vec<TmuxTabConfig> {
    let tabs: Vec<TmuxTabConfig> = config
        .tabs
        .iter()
        .filter_map(|tab| {
            let trimmed = tab.name.trim();
            if trimmed.is_empty() {
                return None;
            }
            let command = tab
                .command
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string());
            Some(TmuxTabConfig {
                name: trimmed.to_string(),
                command,
            })
        })
        .collect();
    if tabs.is_empty() {
        vec![TmuxTabConfig {
            name: DEFAULT_TMUX_TAB.to_string(),
            command: None,
        }]
    } else {
        tabs
    }
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

pub fn rename_session(config: &TmuxConfig, current: &str, next: &str) -> Result<bool, String> {
    if !has_session(config, current)? {
        return Ok(false);
    }
    let output = run_tmux(config, &["rename-session", "-t", current, next])?;
    if output.status.success() {
        Ok(true)
    } else {
        Err(format!(
            "Failed to rename tmux session '{current}' to '{next}'.{}",
            format_output(&output)
        ))
    }
}

pub fn rename_window(config: &TmuxConfig, target: &str, name: &str) -> Result<(), String> {
    let output = run_tmux(config, &["rename-window", "-t", target, name])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to rename tmux window '{target}' to '{name}'.{}",
            format_output(&output)
        ))
    }
}

pub fn new_window(
    config: &TmuxConfig,
    session: &str,
    name: &str,
    cwd: &Path,
    command: Option<&[String]>,
) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let mut args = vec![
        "new-window".to_string(),
        "-t".to_string(),
        session.to_string(),
        "-n".to_string(),
        trimmed.to_string(),
        "-c".to_string(),
        cwd.to_string_lossy().to_string(),
    ];
    if let Some(command) = command {
        args.extend(command.iter().cloned());
    }
    let output = run_tmux_with_args(config, &args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to create tmux window '{trimmed}'.{}",
            format_output(&output)
        ))
    }
}

pub fn send_keys(config: &TmuxConfig, target: &str, command: &str) -> Result<(), String> {
    let output = run_tmux(config, &["send-keys", "-t", target, command, "C-m"])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to send keys to tmux window '{target}'.{}",
            format_output(&output)
        ))
    }
}

pub fn has_session(config: &TmuxConfig, session: &str) -> Result<bool, String> {
    let output = run_tmux(config, &["has-session", "-t", session])?;
    Ok(output.status.success())
}

fn new_session(
    config: &TmuxConfig,
    session: &str,
    cwd: &Path,
    window_name: Option<&str>,
    command: Option<&[String]>,
) -> Result<(), String> {
    let mut args = vec![
        "new-session".to_string(),
        "-d".to_string(),
        "-s".to_string(),
        session.to_string(),
        "-c".to_string(),
        cwd.to_string_lossy().to_string(),
    ];
    if let Some(name) = window_name {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            args.push("-n".to_string());
            args.push(trimmed.to_string());
        }
    }
    if let Some(command) = command {
        args.extend(command.iter().cloned());
    }
    let output = run_tmux_with_args(config, &args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to create tmux session '{session}'.{}",
            format_output(&output)
        ))
    }
}

fn run_tmux(config: &TmuxConfig, args: &[&str]) -> Result<Output, String> {
    let mut cmd = Command::new(resolve_command(config));
    cmd.args(&config.args);
    cmd.args(args);
    cmd.output().map_err(|err| err.to_string())
}

fn run_tmux_with_args(config: &TmuxConfig, args: &[String]) -> Result<Output, String> {
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

fn truecolor_term() -> Option<String> {
    let colorterm = env::var("COLORTERM").ok()?.to_lowercase();
    if !colorterm.contains("truecolor") && !colorterm.contains("24bit") {
        return None;
    }
    match env::var("TERM") {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => None,
    }
}

pub fn setup_shell_body(scripts: &[String]) -> Option<String> {
    let joined = join_setup_commands(scripts)?;
    if cfg!(windows) {
        Some(joined)
    } else {
        Some(format!(
            "set -e; {joined}; echo \"[blackpepper] setup complete\""
        ))
    }
}

pub fn setup_command_args(scripts: &[String]) -> Option<Vec<String>> {
    let joined = join_setup_commands(scripts)?;
    if cfg!(windows) {
        Some(vec!["cmd".to_string(), "/K".to_string(), joined])
    } else {
        let body = format!(
            "set -e; {joined}; echo \"[blackpepper] setup complete\"; exec \"${{SHELL:-sh}}\""
        );
        Some(vec!["sh".to_string(), "-lc".to_string(), body])
    }
}

fn tab_command_args(command: Option<&str>) -> Option<Vec<String>> {
    let command = command?.trim();
    if command.is_empty() {
        return None;
    }
    if cfg!(windows) {
        Some(vec![
            "cmd".to_string(),
            "/K".to_string(),
            command.to_string(),
        ])
    } else {
        let body = format!("set -e; {command}; exec \"${{SHELL:-sh}}\"");
        Some(vec!["sh".to_string(), "-lc".to_string(), body])
    }
}

fn join_setup_commands(scripts: &[String]) -> Option<String> {
    let commands: Vec<String> = scripts
        .iter()
        .map(|script| script.trim())
        .filter(|script| !script.is_empty())
        .map(|script| script.to_string())
        .collect();
    if commands.is_empty() {
        None
    } else {
        Some(commands.join(" && "))
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
