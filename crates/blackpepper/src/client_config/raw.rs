use super::{ConfigError, SshHostConfig, StartupCommand};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawConfig {
    #[serde(default)]
    pub(super) keymap: RawKeymap,
    #[serde(default)]
    pub(super) hosts: BTreeMap<String, SshHostConfig>,
    #[serde(default)]
    pub(super) startup: Vec<StartupCommand>,
    #[serde(default)]
    pub(super) workspace: RawWorkspace,
    #[serde(default)]
    pub(super) ui: RawUi,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawKeymap {
    pub(super) toggle_mode: Option<String>,
    pub(super) switch_workspace: Option<String>,
    pub(super) workspace_overlay: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawWorkspace {
    #[serde(default)]
    pub(super) env: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawUi {
    pub(super) background: Option<String>,
    pub(super) foreground: Option<String>,
    /// The single flag that swaps every non-ASCII glyph for its fallback.
    pub(super) glyphs: Option<String>,
}

pub(super) fn read_optional(path: Option<&Path>) -> Result<Option<RawConfig>, ConfigError> {
    let Some(path) = path else {
        return Ok(None);
    };
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    parse_contents(path, &contents)
}

pub(super) fn parse_optional_contents(
    layer: Option<(PathBuf, String)>,
) -> Result<Option<RawConfig>, ConfigError> {
    let Some((path, contents)) = layer else {
        return Ok(None);
    };
    parse_contents(&path, &contents)
}

fn parse_contents(path: &Path, contents: &str) -> Result<Option<RawConfig>, ConfigError> {
    if contents.trim().is_empty() {
        return Ok(None);
    }
    let value: toml::Value = toml::from_str(contents).map_err(|err| ConfigError::Invalid {
        path: path.to_path_buf(),
        message: err.to_string(),
    })?;
    if value.get("tmux").is_some() {
        return Err(ConfigError::LegacyTmux {
            path: path.to_path_buf(),
        });
    }
    let parsed: RawConfig = toml::from_str(contents).map_err(|err| ConfigError::Invalid {
        path: path.to_path_buf(),
        message: err.to_string(),
    })?;
    validate_raw(path, &parsed)?;
    Ok(Some(parsed))
}

fn validate_raw(path: &Path, raw: &RawConfig) -> Result<(), ConfigError> {
    for (label, value) in [
        ("ui.background", raw.ui.background.as_deref()),
        ("ui.foreground", raw.ui.foreground.as_deref()),
    ] {
        if value.is_some_and(|value| parse_hex_color(value).is_none()) {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                message: format!("{label} must be a six-digit hexadecimal color"),
            });
        }
    }
    if raw
        .ui
        .glyphs
        .as_deref()
        .is_some_and(|value| !matches!(value.trim(), "unicode" | "ascii"))
    {
        return Err(ConfigError::Invalid {
            path: path.to_path_buf(),
            message: "ui.glyphs must be \"unicode\" or \"ascii\"".to_string(),
        });
    }
    for (label, value) in [
        ("keymap.toggle_mode", raw.keymap.toggle_mode.as_deref()),
        (
            "keymap.switch_workspace",
            raw.keymap.switch_workspace.as_deref(),
        ),
        (
            "keymap.workspace_overlay",
            raw.keymap.workspace_overlay.as_deref(),
        ),
    ] {
        if value.is_some_and(|value| crate::keymap::parse_key_chord(value).is_none()) {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                message: format!("{label} is not a valid key chord"),
            });
        }
    }
    for (key, value) in &raw.workspace.env {
        let mut characters = key.chars();
        let valid_key = key.len() <= 128
            && matches!(characters.next(), Some('_' | 'a'..='z' | 'A'..='Z'))
            && characters.all(|character| character == '_' || character.is_ascii_alphanumeric());
        if !valid_key || value.contains('\0') || value.len() > 16 * 1024 {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                message: format!(
                    "workspace.env key {key:?} must be a valid environment name and its value must not exceed 16 KiB"
                ),
            });
        }
    }
    let mut startup_names = std::collections::BTreeSet::new();
    for startup in &raw.startup {
        if startup.name.trim().is_empty()
            || startup.name.len() > 48
            || startup.name.chars().any(char::is_control)
            || startup.command.is_empty()
        {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                message: "each [[startup]] needs a 1-48 character single-line name and a non-empty command array".to_string(),
            });
        }
        if !startup_names.insert(&startup.name) {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                message: format!("duplicate [[startup]] name: {}", startup.name),
            });
        }
        if startup.command.iter().any(|word| word.contains('\0')) {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                message: format!("startup {} contains a NUL byte", startup.name),
            });
        }
    }
    Ok(())
}

pub(super) fn parse_hex_color(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.trim().strip_prefix('#').unwrap_or(value.trim());
    if hex.len() != 6 {
        return None;
    }
    Some((
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ))
}
