use std::env;
use std::path::Path;
use std::process::{Command, Output};

/// Default tabs when none are configured: agent, server, and git.
pub const DEFAULT_TMUX_TABS: &[&str] = &["agent", "server", "git"];
pub const SETUP_TMUX_TAB: &str = "setup";
pub const PR_AGENT_TMUX_TAB: &str = "bp-pr-agent";

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
    format!("{repo}_{workspace}")
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
        ";".to_string(),
        "set-option".to_string(),
        "-gq".to_string(),
        "allow-passthrough".to_string(),
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
    env: &[(String, String)],
) -> Result<bool, String> {
    if has_session(config, session)? {
        set_environment(config, session, env)?;
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
                env,
            )?;
            for tab in tabs {
                let command = tab_command_args(tab.command.as_deref());
                new_window(config, session, &tab.name, cwd, command.as_deref(), env)?;
            }
        }
        None => {
            let (first, rest) = tabs
                .split_first()
                .ok_or_else(|| "No tmux tabs configured.".to_string())?;
            let command = tab_command_args(first.command.as_deref());
            new_session(
                config,
                session,
                cwd,
                Some(&first.name),
                command.as_deref(),
                env,
            )?;
            for tab in rest {
                let command = tab_command_args(tab.command.as_deref());
                new_window(config, session, &tab.name, cwd, command.as_deref(), env)?;
            }
        }
    }

    set_environment(config, session, env)?;
    // Focus the first tab
    select_window(config, &format!("{session}:1"))?;
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
        DEFAULT_TMUX_TABS
            .iter()
            .map(|name| TmuxTabConfig {
                name: name.to_string(),
                command: default_tab_command(name),
            })
            .collect()
    } else {
        tabs
    }
}

/// Returns the default command for a built-in tab name.
fn default_tab_command(name: &str) -> Option<String> {
    match name {
        "git" => Some("gitui".to_string()),
        _ => None,
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

pub fn detach_session(config: &TmuxConfig, session: &str) -> Result<bool, String> {
    if !has_session(config, session)? {
        return Ok(false);
    }
    let output = run_tmux(config, &["detach-client", "-s", session])?;
    if output.status.success() {
        Ok(true)
    } else {
        Err(format!(
            "Failed to detach tmux session '{session}'.{}",
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

pub fn select_window(config: &TmuxConfig, target: &str) -> Result<(), String> {
    let output = run_tmux(config, &["select-window", "-t", target])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to select tmux window '{target}'.{}",
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
    env: &[(String, String)],
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
    for (key, value) in env {
        args.push("-e".to_string());
        args.push(format!("{key}={value}"));
    }
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

pub fn respawn_pane(
    config: &TmuxConfig,
    target: &str,
    cwd: &Path,
    command: &[String],
) -> Result<(), String> {
    if command.is_empty() {
        return Err("No command provided to respawn tmux pane.".to_string());
    }
    let mut args = vec![
        "respawn-pane".to_string(),
        "-k".to_string(),
        "-t".to_string(),
        target.to_string(),
        "-c".to_string(),
        cwd.to_string_lossy().to_string(),
    ];
    args.extend(command.iter().cloned());
    let output = run_tmux_with_args(config, &args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to respawn tmux pane '{target}'.{}",
            format_output(&output)
        ))
    }
}

pub fn set_environment(
    config: &TmuxConfig,
    session: &str,
    env: &[(String, String)],
) -> Result<(), String> {
    for (key, value) in env {
        let output = run_tmux(config, &["set-environment", "-t", session, key, value])?;
        if !output.status.success() {
            return Err(format!(
                "Failed to set tmux environment '{key}'.{}",
                format_output(&output)
            ));
        }
    }
    Ok(())
}

pub fn has_session(config: &TmuxConfig, session: &str) -> Result<bool, String> {
    let output = run_tmux(config, &["has-session", "-t", session])?;
    Ok(output.status.success())
}

pub fn ensure_window(
    config: &TmuxConfig,
    session: &str,
    name: &str,
    cwd: &Path,
) -> Result<String, String> {
    if !has_session(config, session)? {
        return Err(format!("Tmux session '{session}' not found."));
    }
    if let Some(target) = window_target_by_name(config, session, name)? {
        return Ok(target);
    }
    new_window(config, session, name, cwd, None, &[])?;
    window_target_by_name(config, session, name)?
        .ok_or_else(|| format!("Failed to create tmux window '{name}' in session '{session}'."))
}

fn window_target_by_name(
    config: &TmuxConfig,
    session: &str,
    name: &str,
) -> Result<Option<String>, String> {
    let output = run_tmux(
        config,
        &[
            "list-windows",
            "-t",
            session,
            "-F",
            "#{window_name}:#{window_index}",
        ],
    )?;
    if !output.status.success() {
        return Err(format!(
            "Failed to list tmux windows for '{session}'.{}",
            format_output(&output)
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((window, index)) = trimmed.split_once(':') {
            if window == name {
                return Ok(Some(format!("{session}:{index}")));
            }
        }
    }
    Ok(None)
}

fn new_session(
    config: &TmuxConfig,
    session: &str,
    cwd: &Path,
    window_name: Option<&str>,
    command: Option<&[String]>,
    env: &[(String, String)],
) -> Result<(), String> {
    let mut args = vec![
        "new-session".to_string(),
        "-d".to_string(),
        "-s".to_string(),
        session.to_string(),
        "-c".to_string(),
        cwd.to_string_lossy().to_string(),
    ];
    for (key, value) in env {
        args.push("-e".to_string());
        args.push(format!("{key}={value}"));
    }
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
        // Use user's shell with login + interactive flags to source .zshrc/.bashrc
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());
        let body = format!("{command}; exec \"${{SHELL:-sh}}\"");
        Some(vec![shell, "-lic".to_string(), body])
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

#[cfg(test)]
mod tests {
    use super::{default_tab_command, resolve_tabs, DEFAULT_TMUX_TABS};
    use crate::config::TmuxConfig;

    #[test]
    fn default_tabs_includes_git() {
        assert!(DEFAULT_TMUX_TABS.contains(&"git"));
        assert!(DEFAULT_TMUX_TABS.contains(&"agent"));
        assert!(DEFAULT_TMUX_TABS.contains(&"server"));
    }

    #[test]
    fn resolve_tabs_uses_defaults_when_empty() {
        let config = TmuxConfig {
            command: None,
            args: vec![],
            tabs: vec![],
        };
        let tabs = resolve_tabs(&config);
        assert_eq!(tabs.len(), 3);
        assert_eq!(tabs[0].name, "agent");
        assert_eq!(tabs[1].name, "server");
        assert_eq!(tabs[2].name, "git");
    }

    #[test]
    fn git_tab_has_gitui_command() {
        let command = default_tab_command("git");
        assert!(command.is_some(), "git tab should have a default command");
        let cmd = command.unwrap();
        assert!(cmd.contains("gitui"), "git tab command should run gitui");
    }

    #[test]
    fn non_git_tabs_have_no_default_command() {
        assert!(default_tab_command("agent").is_none());
        assert!(default_tab_command("server").is_none());
        assert!(default_tab_command("work").is_none());
    }
}
