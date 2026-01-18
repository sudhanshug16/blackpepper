//! Configuration loading and merging.
//!
//! Config is loaded from three sources with later sources taking precedence:
//! 1. User-level: `~/.config/blackpepper/config.toml`
//! 2. Project-level: `<repo>/.blackpepper/config.toml` (committed)
//! 3. User-project-level: `<repo>/.blackpepper/config.local.toml` (gitignored)
//!
//! Supports keymap customization, tmux command override, workspace root
//! configuration, and workspace setup scripts. Uses TOML format with serde.

use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_TOGGLE_MODE: &str = "ctrl+]";
const DEFAULT_SWITCH_WORKSPACE: &str = "ctrl+\\";
const DEFAULT_WORKSPACE_ROOT: &str = ".blackpepper/workspaces";
const DEFAULT_TMUX_COMMAND: &str = "tmux";
const DEFAULT_GIT_REMOTE: &str = "origin";
const DEFAULT_UI_BG: (u8, u8, u8) = (0x33, 0x33, 0x33);
const DEFAULT_UI_FG: (u8, u8, u8) = (0xff, 0xff, 0xff);

// ============================================================================
// Public config types
// ============================================================================

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
    pub env: Vec<(String, String)>,
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

// ============================================================================
// Raw deserialization types
// ============================================================================

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
    tabs: Option<IndexMap<String, RawTmuxTab>>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTmuxTab {
    command: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawWorkspace {
    root: Option<String>,
    setup: Option<RawWorkspaceSetup>,
    env: Option<BTreeMap<String, String>>,
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

// ============================================================================
// Config layer helpers
// ============================================================================

/// Three config layers: user (lowest priority), project, local (highest priority).
struct Layers {
    user: Option<RawConfig>,
    project: Option<RawConfig>,
    local: Option<RawConfig>,
}

impl Layers {
    /// Resolve a value from the three layers, highest priority first.
    fn resolve<T, F>(&self, mut extract: F) -> Option<T>
    where
        F: FnMut(&RawConfig) -> Option<T>,
    {
        self.local
            .as_ref()
            .and_then(&mut extract)
            .or_else(|| self.project.as_ref().and_then(&mut extract))
            .or_else(|| self.user.as_ref().and_then(&mut extract))
    }

    /// Resolve a value with a default fallback.
    fn resolve_or<T, F>(&self, extract: F, default: T) -> T
    where
        F: FnMut(&RawConfig) -> Option<T>,
    {
        self.resolve(extract).unwrap_or(default)
    }

    /// Resolve a string value with a default.
    fn resolve_string<F>(&self, extract: F, default: &str) -> String
    where
        F: FnMut(&RawConfig) -> Option<String>,
    {
        self.resolve(extract).unwrap_or_else(|| default.to_string())
    }

    /// Merge env vars from all layers (user first, then project, then local overrides).
    fn merge_env(&self) -> Vec<(String, String)> {
        let mut env = BTreeMap::new();
        for layer in [&self.user, &self.project, &self.local] {
            if let Some(layer_env) = layer
                .as_ref()
                .and_then(|c| c.workspace.as_ref())
                .and_then(|w| w.env.as_ref())
            {
                env.extend(layer_env.clone());
            }
        }
        env.into_iter().collect()
    }
}

// ============================================================================
// Config loading and merging
// ============================================================================

fn read_toml(path: &Path) -> Option<RawConfig> {
    let contents = fs::read_to_string(path).ok()?;
    if contents.trim().is_empty() {
        return None;
    }
    toml::from_str::<RawConfig>(&contents).ok()
}

fn merge_config(
    user: Option<RawConfig>,
    project: Option<RawConfig>,
    local: Option<RawConfig>,
) -> Config {
    let layers = Layers {
        user,
        project,
        local,
    };

    Config {
        keymap: KeymapConfig {
            toggle_mode: layers.resolve_string(
                |c| c.keymap.as_ref()?.toggle_mode.clone(),
                DEFAULT_TOGGLE_MODE,
            ),
            switch_workspace: layers.resolve_string(
                |c| c.keymap.as_ref()?.switch_workspace.clone(),
                DEFAULT_SWITCH_WORKSPACE,
            ),
        },
        tmux: TmuxConfig {
            command: layers
                .resolve(|c| c.tmux.as_ref()?.command.clone())
                .or_else(|| Some(DEFAULT_TMUX_COMMAND.to_string())),
            args: layers.resolve_or(|c| c.tmux.as_ref()?.args.clone(), vec![]),
            tabs: layers
                .resolve(|c| c.tmux.as_ref()?.tabs.as_ref().map(collect_tmux_tabs))
                .unwrap_or_default(),
        },
        workspace: WorkspaceConfig {
            root: PathBuf::from(layers.resolve_string(
                |c| c.workspace.as_ref()?.root.clone(),
                DEFAULT_WORKSPACE_ROOT,
            )),
            setup_scripts: layers.resolve_or(
                |c| c.workspace.as_ref()?.setup.as_ref()?.scripts.clone(),
                vec![],
            ),
            env: layers.merge_env(),
        },
        git: GitConfig {
            remote: layers.resolve_string(|c| c.git.as_ref()?.remote.clone(), DEFAULT_GIT_REMOTE),
        },
        agent: AgentConfig {
            provider: layers.resolve(|c| c.agent.as_ref()?.provider.clone()),
            command: layers.resolve(|c| c.agent.as_ref()?.command.clone()),
        },
        upstream: UpstreamConfig {
            provider: layers.resolve_string(|c| c.upstream.as_ref()?.provider.clone(), "github"),
        },
        ui: UiConfig {
            background: layers
                .resolve(|c| parse_hex_color(c.ui.as_ref()?.background.as_deref()?))
                .unwrap_or(DEFAULT_UI_BG),
            foreground: layers
                .resolve(|c| parse_hex_color(c.ui.as_ref()?.foreground.as_deref()?))
                .unwrap_or(DEFAULT_UI_FG),
        },
    }
}

fn collect_tmux_tabs(tabs: &IndexMap<String, RawTmuxTab>) -> Vec<TmuxTabConfig> {
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

// ============================================================================
// Public API
// ============================================================================

pub fn workspace_config_path(root: &Path) -> PathBuf {
    root.join(".blackpepper").join("config.toml")
}

pub fn workspace_local_config_path(root: &Path) -> PathBuf {
    root.join(".blackpepper").join("config.local.toml")
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
    let project_path = workspace_config_path(root);
    let local_path = workspace_local_config_path(root);
    let user_path = user_config_path();

    let project_config = read_toml(&project_path);
    let local_config = read_toml(&local_path);
    let user_config = user_path.and_then(|path| read_toml(&path));

    merge_config(user_config, project_config, local_config)
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
    let output =
        toml::to_string_pretty(&value).map_err(|err| std::io::Error::other(err.to_string()))?;
    fs::write(path, output)?;
    Ok(())
}

#[cfg(test)]
mod tests;
