//! Persistent application state.
//!
//! Stores state that persists across sessions in:
//! `~/.config/blackpepper/state.toml`
//!
//! Tracks active workspaces per repository and allocates workspace
//! port blocks for tmux sessions.

use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const PORT_RANGE_START: u16 = 30000;
const PORT_RANGE_END: u16 = 39999;
pub const PORT_BLOCK_SIZE: u16 = 10;

#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub active_workspaces: HashMap<String, String>,
    pub workspace_ports: HashMap<String, u16>,
}

#[derive(Debug, Deserialize)]
struct RawState {
    #[serde(alias = "activeWorkspaces")]
    active_workspaces: Option<HashMap<String, String>>,
    #[serde(alias = "workspacePorts")]
    workspace_ports: Option<HashMap<String, u16>>,
    #[serde(alias = "last_workspace_path")]
    #[allow(dead_code)]
    last_workspace_path: Option<String>,
    #[serde(alias = "lastWorkspacePath")]
    #[allow(dead_code)]
    last_workspace_path_alt: Option<String>,
    #[serde(alias = "last_path")]
    #[allow(dead_code)]
    last_path: Option<String>,
    #[serde(alias = "lastPath")]
    #[allow(dead_code)]
    last_path_alt: Option<String>,
}

fn state_path() -> Option<PathBuf> {
    if let Ok(override_path) = env::var("BLACKPEPPER_STATE_PATH") {
        let trimmed = override_path.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    let home = dirs::home_dir()?;
    Some(home.join(".config").join("blackpepper").join("state.toml"))
}

#[cfg(test)]
mod tests;

fn canonicalize_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn normalize_workspace_map(raw: &HashMap<String, String>) -> HashMap<String, String> {
    let mut normalized = HashMap::new();
    for (key, value) in raw {
        let key_path = canonicalize_path(Path::new(key));
        let value_path = canonicalize_path(Path::new(value));
        normalized.insert(
            key_path.to_string_lossy().to_string(),
            value_path.to_string_lossy().to_string(),
        );
    }
    normalized
}

fn normalize_port_map(raw: &HashMap<String, u16>) -> HashMap<String, u16> {
    let mut normalized = HashMap::new();
    let mut used = HashSet::new();
    for (key, value) in raw {
        if !valid_port_base(*value) || used.contains(value) {
            continue;
        }
        let key_path = canonicalize_path(Path::new(key));
        normalized.insert(key_path.to_string_lossy().to_string(), *value);
        used.insert(*value);
    }
    normalized
}

pub fn load_state() -> Option<AppState> {
    let path = state_path()?;
    let contents = fs::read_to_string(path).ok()?;
    if contents.trim().is_empty() {
        return None;
    }

    let raw: RawState = toml::from_str(&contents).ok()?;
    let active = raw.active_workspaces.unwrap_or_default();
    let ports = raw.workspace_ports.unwrap_or_default();
    if active.is_empty() && ports.is_empty() {
        return None;
    }

    Some(AppState {
        active_workspaces: normalize_workspace_map(&active),
        workspace_ports: normalize_port_map(&ports),
    })
}

pub fn save_state(state: &AppState) -> std::io::Result<()> {
    let path = match state_path() {
        Some(path) => path,
        None => return Ok(()),
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut contents = String::new();
    if !state.active_workspaces.is_empty() {
        let mut entries: Vec<_> = state.active_workspaces.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));

        contents.push_str("[active_workspaces]\n");
        for (key, value) in entries {
            let encoded_key = toml::Value::String(key.to_string()).to_string();
            let encoded_value = toml::Value::String(value.to_string()).to_string();
            contents.push_str(&format!("{encoded_key} = {encoded_value}\n"));
        }
    }

    if !state.workspace_ports.is_empty() {
        if !contents.is_empty() {
            contents.push('\n');
        }
        let mut entries: Vec<_> = state.workspace_ports.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));

        contents.push_str("[workspace_ports]\n");
        for (key, value) in entries {
            let encoded_key = toml::Value::String(key.to_string()).to_string();
            let encoded_value = toml::Value::Integer((*value).into()).to_string();
            contents.push_str(&format!("{encoded_key} = {encoded_value}\n"));
        }
    }

    fs::write(path, contents)
}

pub fn record_active_workspace(repo_root: &Path, workspace_path: &Path) -> std::io::Result<()> {
    if repo_root.as_os_str().is_empty() || workspace_path.as_os_str().is_empty() {
        return Ok(());
    }
    let normalized_root = canonicalize_path(repo_root);
    let normalized_workspace = canonicalize_path(workspace_path);

    let mut state = load_state().unwrap_or_default();
    state.active_workspaces.insert(
        normalized_root.to_string_lossy().to_string(),
        normalized_workspace.to_string_lossy().to_string(),
    );
    save_state(&state)
}

pub fn remove_active_workspace(repo_root: &Path) -> std::io::Result<()> {
    if repo_root.as_os_str().is_empty() {
        return Ok(());
    }
    let normalized_root = canonicalize_path(repo_root);
    let key = normalized_root.to_string_lossy().to_string();

    let mut state = load_state().unwrap_or_default();
    if state.active_workspaces.remove(&key).is_some() {
        save_state(&state)?;
    }
    Ok(())
}

pub fn get_active_workspace(state: &AppState, repo_root: &Path) -> Option<PathBuf> {
    let normalized_root = canonicalize_path(repo_root);
    let key = normalized_root.to_string_lossy();
    state.active_workspaces.get(key.as_ref()).map(PathBuf::from)
}

pub fn ensure_workspace_ports(workspace_path: &Path) -> io::Result<u16> {
    if workspace_path.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Workspace path is empty.",
        ));
    }
    let key = canonicalize_path(workspace_path)
        .to_string_lossy()
        .to_string();

    let mut state = load_state().unwrap_or_default();
    if let Some(base) = state.workspace_ports.get(&key) {
        return Ok(*base);
    }

    let used: HashSet<u16> = state.workspace_ports.values().copied().collect();
    let base = next_available_port_base(&used)?;
    state.workspace_ports.insert(key, base);
    save_state(&state)?;
    Ok(base)
}

pub fn remove_workspace_ports(workspace_path: &Path) -> io::Result<()> {
    if workspace_path.as_os_str().is_empty() {
        return Ok(());
    }
    let key = canonicalize_path(workspace_path)
        .to_string_lossy()
        .to_string();
    let mut state = load_state().unwrap_or_default();
    if state.workspace_ports.remove(&key).is_some() {
        save_state(&state)?;
    }
    Ok(())
}

pub fn rename_workspace_ports(old_path: &Path, new_path: &Path) -> io::Result<()> {
    if old_path.as_os_str().is_empty() || new_path.as_os_str().is_empty() {
        return Ok(());
    }
    let old_key = canonicalize_path(old_path).to_string_lossy().to_string();
    let new_key = canonicalize_path(new_path).to_string_lossy().to_string();
    let mut state = load_state().unwrap_or_default();
    if let Some(base) = state.workspace_ports.remove(&old_key) {
        state.workspace_ports.insert(new_key, base);
        save_state(&state)?;
    }
    Ok(())
}

pub fn workspace_port_env(base: u16) -> Vec<(String, String)> {
    (0..PORT_BLOCK_SIZE)
        .map(|offset| {
            (
                format!("WORKSPACE_PORT_{offset}"),
                (base + offset).to_string(),
            )
        })
        .collect()
}

fn valid_port_base(base: u16) -> bool {
    if base < PORT_RANGE_START {
        return false;
    }
    if base > PORT_RANGE_END {
        return false;
    }
    let max = match base.checked_add(PORT_BLOCK_SIZE - 1) {
        Some(max) => max,
        None => return false,
    };
    if max > PORT_RANGE_END {
        return false;
    }
    (base - PORT_RANGE_START) % PORT_BLOCK_SIZE == 0
}

fn next_available_port_base(used: &HashSet<u16>) -> io::Result<u16> {
    let mut base = PORT_RANGE_START;
    while base <= PORT_RANGE_END {
        if valid_port_base(base) && !used.contains(&base) {
            return Ok(base);
        }
        match base.checked_add(PORT_BLOCK_SIZE) {
            Some(next) => base = next,
            None => break,
        }
    }
    Err(io::Error::new(
        io::ErrorKind::Other,
        "No available workspace port blocks remaining.",
    ))
}
