//! Configuration loading and merging.
//!
//! Config is loaded from two sources with workspace taking precedence:
//! 1. User-level: `~/.config/blackpepper/config.toml`
//! 2. Workspace-level: `<repo>/.config/blackpepper/config.toml`
//!
//! Supports keymap customization, tmux command override, workspace root
//! configuration, and workspace setup scripts. Uses TOML format with serde.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_TOGGLE_MODE: &str = "ctrl+]";
const DEFAULT_SWITCH_WORKSPACE: &str = "ctrl+p";
const DEFAULT_WORKSPACE_ROOT: &str = ".blackpepper/workspaces";
const DEFAULT_TMUX_COMMAND: &str = "tmux";
const DEFAULT_GIT_REMOTE: &str = "origin";
const DEFAULT_UI_BG: (u8, u8, u8) = (0x33, 0x33, 0x33);
const DEFAULT_UI_FG: (u8, u8, u8) = (0xff, 0xff, 0xff);

#[derive(Debug, Clone)]
pub struct Config {
    pub keymap: KeymapConfig,
    pub tmux: TmuxConfig,
    pub workspace: WorkspaceConfig,
    pub git: GitConfig,
    pub agent: AgentConfig,
    pub upstream: UpstreamConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Clone)]
pub struct KeymapConfig {
    pub toggle_mode: String,
    pub switch_workspace: String,
}

#[derive(Debug, Clone)]
pub struct TmuxConfig {
    pub command: Option<String>,
    pub args: Vec<String>,
    pub tabs: Vec<TmuxTabConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxTabConfig {
    pub name: String,
    pub command: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceConfig {
    pub root: PathBuf,
    pub setup_scripts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub provider: Option<String>,
    pub command: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UpstreamConfig {
    pub provider: String,
}

#[derive(Debug, Clone)]
pub struct GitConfig {
    pub remote: String,
}

#[derive(Debug, Clone, Copy)]
pub struct UiConfig {
    pub background: (u8, u8, u8),
    pub foreground: (u8, u8, u8),
}

#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    keymap: Option<RawKeymap>,
    tmux: Option<RawTmux>,
    workspace: Option<RawWorkspace>,
    git: Option<RawGit>,
    agent: Option<RawAgent>,
    upstream: Option<RawUpstream>,
    ui: Option<RawUi>,
}

#[derive(Debug, Default, Deserialize)]
struct RawKeymap {
    #[serde(alias = "toggleMode")]
    toggle_mode: Option<String>,
    #[serde(alias = "switchWorkspace")]
    switch_workspace: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTmux {
    command: Option<String>,
    args: Option<Vec<String>>,
    tabs: Option<BTreeMap<String, RawTmuxTab>>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTmuxTab {
    command: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawWorkspace {
    root: Option<String>,
    setup: Option<RawWorkspaceSetup>,
}

#[derive(Debug, Default, Deserialize)]
struct RawWorkspaceSetup {
    scripts: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
struct RawAgent {
    provider: Option<String>,
    command: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawUpstream {
    provider: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawGit {
    remote: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawUi {
    background: Option<String>,
    foreground: Option<String>,
}

fn read_toml(path: &Path) -> Option<RawConfig> {
    let contents = fs::read_to_string(path).ok()?;
    if contents.trim().is_empty() {
        return None;
    }
    toml::from_str::<RawConfig>(&contents).ok()
}

fn merge_config(user: Option<RawConfig>, workspace: Option<RawConfig>) -> Config {
    let workspace_keymap = workspace.as_ref().and_then(|c| c.keymap.as_ref());
    let user_keymap = user.as_ref().and_then(|c| c.keymap.as_ref());
    let toggle_mode = workspace_keymap
        .and_then(|k| k.toggle_mode.clone())
        .or_else(|| user_keymap.and_then(|k| k.toggle_mode.clone()))
        .unwrap_or_else(|| DEFAULT_TOGGLE_MODE.to_string());
    let switch_workspace = workspace_keymap
        .and_then(|k| k.switch_workspace.clone())
        .or_else(|| user_keymap.and_then(|k| k.switch_workspace.clone()))
        .unwrap_or_else(|| DEFAULT_SWITCH_WORKSPACE.to_string());

    let workspace_tmux = workspace.as_ref().and_then(|c| c.tmux.as_ref());
    let user_tmux = user.as_ref().and_then(|c| c.tmux.as_ref());
    let workspace_workspace = workspace.as_ref().and_then(|c| c.workspace.as_ref());
    let user_workspace = user.as_ref().and_then(|c| c.workspace.as_ref());
    let workspace_agent = workspace.as_ref().and_then(|c| c.agent.as_ref());
    let user_agent = user.as_ref().and_then(|c| c.agent.as_ref());
    let workspace_git = workspace.as_ref().and_then(|c| c.git.as_ref());
    let user_git = user.as_ref().and_then(|c| c.git.as_ref());
    let workspace_upstream = workspace.as_ref().and_then(|c| c.upstream.as_ref());
    let user_upstream = user.as_ref().and_then(|c| c.upstream.as_ref());
    let workspace_ui = workspace.as_ref().and_then(|c| c.ui.as_ref());
    let user_ui = user.as_ref().and_then(|c| c.ui.as_ref());

    let command = workspace_tmux
        .and_then(|t| t.command.clone())
        .or_else(|| user_tmux.and_then(|t| t.command.clone()))
        .or_else(|| Some(DEFAULT_TMUX_COMMAND.to_string()));
    let args = workspace_tmux
        .and_then(|t| t.args.clone())
        .or_else(|| user_tmux.and_then(|t| t.args.clone()))
        .unwrap_or_default();
    let tabs = workspace_tmux
        .and_then(|t| t.tabs.as_ref())
        .or_else(|| user_tmux.and_then(|t| t.tabs.as_ref()))
        .map(collect_tmux_tabs)
        .unwrap_or_default();
    let workspace_root = workspace_workspace
        .and_then(|w| w.root.clone())
        .or_else(|| user_workspace.and_then(|w| w.root.clone()))
        .unwrap_or_else(|| DEFAULT_WORKSPACE_ROOT.to_string());
    let workspace_setup_scripts = workspace_workspace
        .and_then(|workspace| workspace.setup.as_ref())
        .and_then(|setup| setup.scripts.clone())
        .or_else(|| {
            user_workspace
                .and_then(|workspace| workspace.setup.as_ref())
                .and_then(|setup| setup.scripts.clone())
        })
        .unwrap_or_default();
    let agent_provider = workspace_agent
        .and_then(|agent| agent.provider.clone())
        .or_else(|| user_agent.and_then(|agent| agent.provider.clone()));
    let agent_command = workspace_agent
        .and_then(|agent| agent.command.clone())
        .or_else(|| user_agent.and_then(|agent| agent.command.clone()));
    let upstream_provider = workspace_upstream
        .and_then(|upstream| upstream.provider.clone())
        .or_else(|| user_upstream.and_then(|upstream| upstream.provider.clone()))
        .unwrap_or_else(|| "github".to_string());
    let git_remote = workspace_git
        .and_then(|git| git.remote.clone())
        .or_else(|| user_git.and_then(|git| git.remote.clone()))
        .unwrap_or_else(|| DEFAULT_GIT_REMOTE.to_string());
    let ui_background = parse_ui_color(
        workspace_ui.and_then(|ui| ui.background.clone()),
        user_ui.and_then(|ui| ui.background.clone()),
        DEFAULT_UI_BG,
    );
    let ui_foreground = parse_ui_color(
        workspace_ui.and_then(|ui| ui.foreground.clone()),
        user_ui.and_then(|ui| ui.foreground.clone()),
        DEFAULT_UI_FG,
    );

    Config {
        keymap: KeymapConfig {
            toggle_mode,
            switch_workspace,
        },
        tmux: TmuxConfig {
            command,
            args,
            tabs,
        },
        workspace: WorkspaceConfig {
            root: PathBuf::from(workspace_root),
            setup_scripts: workspace_setup_scripts,
        },
        git: GitConfig { remote: git_remote },
        agent: AgentConfig {
            provider: agent_provider,
            command: agent_command,
        },
        upstream: UpstreamConfig {
            provider: upstream_provider,
        },
        ui: UiConfig {
            background: ui_background,
            foreground: ui_foreground,
        },
    }
}

fn collect_tmux_tabs(tabs: &BTreeMap<String, RawTmuxTab>) -> Vec<TmuxTabConfig> {
    tabs.iter()
        .filter_map(|(name, tab)| {
            let trimmed = name.trim();
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
        .collect()
}

fn parse_ui_color(
    workspace_value: Option<String>,
    user_value: Option<String>,
    default_value: (u8, u8, u8),
) -> (u8, u8, u8) {
    workspace_value
        .as_deref()
        .and_then(parse_hex_color)
        .or_else(|| user_value.as_deref().and_then(parse_hex_color))
        .unwrap_or(default_value)
}

fn parse_hex_color(value: &str) -> Option<(u8, u8, u8)> {
    let trimmed = value.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
            Some((r * 17, g * 17, b * 17))
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some((r, g, b))
        }
        _ => None,
    }
}

pub fn workspace_config_path(root: &Path) -> PathBuf {
    root.join(".config").join("blackpepper").join("config.toml")
}

pub fn user_config_path() -> Option<PathBuf> {
    if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
        if !config_home.trim().is_empty() {
            return Some(
                PathBuf::from(config_home)
                    .join("blackpepper")
                    .join("config.toml"),
            );
        }
    }
    let config_root = dirs::config_dir()?;
    Some(config_root.join("blackpepper").join("config.toml"))
}

pub fn load_config(root: &Path) -> Config {
    let workspace_path = workspace_config_path(root);
    let user_path = user_config_path();

    let workspace_config = read_toml(&workspace_path);
    let user_config = user_path.and_then(|path| read_toml(&path));

    merge_config(user_config, workspace_config)
}

pub fn save_user_agent_provider(provider: &str) -> std::io::Result<()> {
    let path = user_config_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Unable to resolve user config directory.",
        )
    })?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut value = if path.exists() {
        let contents = fs::read_to_string(&path)?;
        toml::from_str::<toml::Value>(&contents)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?
    } else {
        toml::Value::Table(Default::default())
    };
    let table = value.as_table_mut().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid config format.")
    })?;
    let agent_entry = table
        .entry("agent")
        .or_insert_with(|| toml::Value::Table(Default::default()));
    let agent_table = agent_entry.as_table_mut().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Invalid agent config format.",
        )
    })?;
    agent_table.insert(
        "provider".to_string(),
        toml::Value::String(provider.to_string()),
    );
    let output = toml::to_string_pretty(&value)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err.to_string()))?;
    fs::write(path, output)?;
    Ok(())
}

#[cfg(test)]
mod tests;
