use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_TOGGLE_MODE: &str = "ctrl+g";
const DEFAULT_SWITCH_WORKSPACE: &str = "ctrl+p";
const DEFAULT_SWITCH_TAB: &str = "ctrl+o";

#[derive(Debug, Clone)]
pub struct Config {
    pub keymap: KeymapConfig,
    pub terminal: TerminalConfig,
}

#[derive(Debug, Clone)]
pub struct KeymapConfig {
    pub toggle_mode: String,
    pub switch_workspace: String,
    pub switch_tab: String,
}

#[derive(Debug, Clone)]
pub struct TerminalConfig {
    pub command: Option<String>,
    pub args: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    keymap: Option<RawKeymap>,
    terminal: Option<RawTerminal>,
}

#[derive(Debug, Default, Deserialize)]
struct RawKeymap {
    #[serde(alias = "toggleMode")]
    toggle_mode: Option<String>,
    #[serde(alias = "switchWorkspace")]
    switch_workspace: Option<String>,
    #[serde(alias = "switchTab")]
    switch_tab: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTerminal {
    command: Option<String>,
    args: Option<Vec<String>>,
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
    let switch_tab = workspace_keymap
        .and_then(|k| k.switch_tab.clone())
        .or_else(|| user_keymap.and_then(|k| k.switch_tab.clone()))
        .unwrap_or_else(|| DEFAULT_SWITCH_TAB.to_string());

    let workspace_terminal = workspace.as_ref().and_then(|c| c.terminal.as_ref());
    let user_terminal = user.as_ref().and_then(|c| c.terminal.as_ref());

    let command = workspace_terminal
        .and_then(|t| t.command.clone())
        .or_else(|| user_terminal.and_then(|t| t.command.clone()));
    let args = workspace_terminal
        .and_then(|t| t.args.clone())
        .or_else(|| user_terminal.and_then(|t| t.args.clone()))
        .unwrap_or_default();

    Config {
        keymap: KeymapConfig {
            toggle_mode,
            switch_workspace,
            switch_tab,
        },
        terminal: TerminalConfig { command, args },
    }
}

fn config_path_from_root(root: &Path) -> PathBuf {
    root.join(".config").join("blackpepper").join("pepper.toml")
}

fn user_config_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(config_path_from_root(&home))
}

pub fn load_config(cwd: &Path) -> Config {
    let workspace_path = config_path_from_root(cwd);
    let user_path = user_config_path();

    let workspace_config = read_toml(&workspace_path);
    let user_config = user_path.and_then(|path| read_toml(&path));

    merge_config(user_config, workspace_config)
}
