//! Strict V1 configuration. Legacy tmux keys are rejected with a migration
//! message; parse errors never fall back to defaults silently.

mod raw;

use raw::{parse_hex_color, parse_optional_contents, read_optional, RawConfig};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

const DEFAULT_TOGGLE_MODE: &str = "ctrl+]";
const DEFAULT_SWITCH_WORKSPACE: &str = "ctrl+n";
const DEFAULT_WORKSPACE_OVERLAY: &str = "ctrl+\\";

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub keymap: KeymapConfig,
    pub hosts: BTreeMap<String, SshHostConfig>,
    pub startup: Vec<StartupCommand>,
    pub workspace_env: BTreeMap<String, String>,
    pub ui: UiConfig,
}

#[derive(Debug, Clone)]
pub struct KeymapConfig {
    pub toggle_mode: String,
    pub switch_workspace: String,
    pub workspace_overlay: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SshHostConfig {
    /// OpenSSH Host alias. Keeping this as the destination preserves Include,
    /// Match, agent, ProxyJump, and platform keychain behavior.
    #[serde(default)]
    pub destination: Option<String>,
}

impl SshHostConfig {
    pub fn destination<'a>(&'a self, name: &'a str) -> &'a str {
        self.destination.as_deref().unwrap_or(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartupCommand {
    pub name: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub auto_start: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiConfig {
    pub background: (u8, u8, u8),
    pub foreground: (u8, u8, u8),
    pub color_tier: ColorTier,
    pub glyphs: GlyphSet,
}

/// Which repertoire the renderer may draw from. The layout, column widths, and
/// status words are identical in both sets, so switching can only change the
/// shape of a marker, never what the client is claiming.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GlyphSet {
    #[default]
    Unicode,
    Ascii,
}

/// Terminal paint capability. Layout and public status words never vary by
/// this tier, so losing color cannot hide state or move controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorTier {
    TrueColor,
    Ansi256,
    Ansi16,
    NoColor,
}

impl ColorTier {
    fn from_environment() -> Self {
        if std::env::var_os("NO_COLOR").is_some() {
            return Self::NoColor;
        }
        if std::env::var("COLORTERM").is_ok_and(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("truecolor") || value.contains("24bit")
        }) {
            return Self::TrueColor;
        }
        if std::env::var("TERM").is_ok_and(|value| value.to_ascii_lowercase().contains("256color"))
        {
            return Self::Ansi256;
        }
        Self::Ansi16
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Invalid {
        path: PathBuf,
        message: String,
    },
    LegacyTmux {
        path: PathBuf,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(formatter, "Could not read {}: {source}", path.display())
            }
            Self::Invalid { path, message } => {
                write!(formatter, "Invalid Blackpepper config {}: {message}", path.display())
            }
            Self::LegacyTmux { path } => write!(
                formatter,
                "Legacy [tmux] configuration found in {}. Blackpepper V1 uses Zellij and starts one shell by default. Move startup commands to [[startup]] entries and remove [tmux.*]. The old file was not changed.",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

pub fn load(root: &Path) -> Result<ClientConfig, ConfigError> {
    let user_path = user_config_path();
    let project_path = root.join(".blackpepper").join("config.toml");
    let local_path = root.join(".blackpepper").join("config.local.toml");

    let user = read_optional(user_path.as_deref())?;
    let project = read_optional(Some(&project_path))?;
    let local = read_optional(Some(&local_path))?;
    Ok(merge(user, project, local))
}

pub(crate) fn load_contents(
    user: Option<(PathBuf, String)>,
    project: Option<(PathBuf, String)>,
    local: Option<(PathBuf, String)>,
) -> Result<ClientConfig, ConfigError> {
    let user = parse_optional_contents(user)?;
    let project = parse_optional_contents(project)?;
    let local = parse_optional_contents(local)?;
    Ok(merge(user, project, local))
}

pub fn user_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|root| root.join("blackpepper").join("config.toml"))
}

fn merge(
    user: Option<RawConfig>,
    project: Option<RawConfig>,
    local: Option<RawConfig>,
) -> ClientConfig {
    let layers = [&user, &project, &local];
    let resolve = |getter: fn(&RawConfig) -> Option<String>, default: &str| {
        layers
            .iter()
            .rev()
            .find_map(|layer| layer.as_ref().and_then(getter))
            .unwrap_or_else(|| default.to_string())
    };
    let mut env = BTreeMap::new();
    for layer in layers.iter().filter_map(|layer| layer.as_ref()) {
        env.extend(layer.workspace.env.clone());
    }
    let startup = layers
        .iter()
        .rev()
        .find_map(|layer| {
            layer
                .as_ref()
                .filter(|config| !config.startup.is_empty())
                .map(|config| config.startup.clone())
        })
        .unwrap_or_default();
    let hosts = user
        .as_ref()
        .map(|raw| raw.hosts.clone())
        .unwrap_or_default();
    ClientConfig {
        keymap: KeymapConfig {
            toggle_mode: resolve(|raw| raw.keymap.toggle_mode.clone(), DEFAULT_TOGGLE_MODE),
            switch_workspace: resolve(
                |raw| raw.keymap.switch_workspace.clone(),
                DEFAULT_SWITCH_WORKSPACE,
            ),
            workspace_overlay: resolve(
                |raw| raw.keymap.workspace_overlay.clone(),
                DEFAULT_WORKSPACE_OVERLAY,
            ),
        },
        hosts,
        startup,
        workspace_env: env,
        ui: UiConfig {
            background: resolve_color(
                &layers,
                |raw| raw.ui.background.as_deref(),
                (0x1c, 0x1d, 0x1f),
            ),
            foreground: resolve_color(
                &layers,
                |raw| raw.ui.foreground.as_deref(),
                (0xe6, 0xe4, 0xe1),
            ),
            color_tier: ColorTier::from_environment(),
            glyphs: layers
                .iter()
                .rev()
                .find_map(|layer| layer.as_ref().and_then(|raw| raw.ui.glyphs.as_deref()))
                .map(|value| match value.trim() {
                    "ascii" => GlyphSet::Ascii,
                    _ => GlyphSet::Unicode,
                })
                .unwrap_or_default(),
        },
    }
}

fn resolve_color(
    layers: &[&Option<RawConfig>],
    getter: fn(&RawConfig) -> Option<&str>,
    default: (u8, u8, u8),
) -> (u8, u8, u8) {
    layers
        .iter()
        .rev()
        .find_map(|layer| layer.as_ref().and_then(getter).and_then(parse_hex_color))
        .unwrap_or(default)
}

#[cfg(test)]
mod tests;
