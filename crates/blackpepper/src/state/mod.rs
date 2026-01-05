//! Persistent application state.
//!
//! Stores state that persists across sessions in:
//! `~/.config/blackpepper/state.toml`
//!
//! Currently tracks which workspace was last active per repository,
//! allowing the app to restore the previous context on startup.

use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub active_workspaces: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct RawState {
    #[serde(alias = "activeWorkspaces")]
    active_workspaces: Option<HashMap<String, String>>,
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
    let home = dirs::home_dir()?;
    Some(home.join(".config").join("blackpepper").join("state.toml"))
}

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

pub fn load_state() -> Option<AppState> {
    let path = state_path()?;
    let contents = fs::read_to_string(path).ok()?;
    if contents.trim().is_empty() {
        return None;
    }

    let raw: RawState = toml::from_str(&contents).ok()?;
    let map = raw.active_workspaces.unwrap_or_default();
    if map.is_empty() {
        return None;
    }

    Some(AppState {
        active_workspaces: normalize_workspace_map(&map),
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

    let mut entries: Vec<_> = state.active_workspaces.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));

    let mut contents = String::from("[active_workspaces]\n");
    for (key, value) in entries {
        let encoded_key = toml::Value::String(key.to_string()).to_string();
        let encoded_value = toml::Value::String(value.to_string()).to_string();
        contents.push_str(&format!("{encoded_key} = {encoded_value}\n"));
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
