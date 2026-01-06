//! Configuration loading and merging.
//!
//! Config is loaded from two sources with workspace taking precedence:
//! 1. User-level: `~/.blackpepper/config.toml`
//! 2. Workspace-level: `<repo>/.blackpepper/config.toml`
//!
//! Supports keymap customization, terminal command override, and
//! workspace root configuration. Uses TOML format with serde.

use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_TOGGLE_MODE: &str = "ctrl+g";
const DEFAULT_SWITCH_WORKSPACE: &str = "ctrl+p";
const DEFAULT_SWITCH_TAB: &str = "ctrl+o";
const DEFAULT_WORKSPACE_ROOT: &str = ".blackpepper/workspaces";
const DEFAULT_REFRESH: &str = "ctrl+r";

#[derive(Debug, Clone)]
pub struct Config {
    pub keymap: KeymapConfig,
    pub terminal: TerminalConfig,
    pub workspace: WorkspaceConfig,
}

#[derive(Debug, Clone)]
pub struct KeymapConfig {
    pub toggle_mode: String,
    pub switch_workspace: String,
    pub switch_tab: String,
    pub refresh: String,
}

#[derive(Debug, Clone)]
pub struct TerminalConfig {
    pub command: Option<String>,
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceConfig {
    pub root: PathBuf,
}

#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    keymap: Option<RawKeymap>,
    terminal: Option<RawTerminal>,
    workspace: Option<RawWorkspace>,
}

#[derive(Debug, Default, Deserialize)]
struct RawKeymap {
    #[serde(alias = "toggleMode")]
    toggle_mode: Option<String>,
    #[serde(alias = "switchWorkspace")]
    switch_workspace: Option<String>,
    #[serde(alias = "switchTab")]
    switch_tab: Option<String>,
    #[serde(alias = "refreshUi")]
    refresh: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTerminal {
    command: Option<String>,
    args: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
struct RawWorkspace {
    root: Option<String>,
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
    let refresh = workspace_keymap
        .and_then(|k| k.refresh.clone())
        .or_else(|| user_keymap.and_then(|k| k.refresh.clone()))
        .unwrap_or_else(|| DEFAULT_REFRESH.to_string());

    let workspace_terminal = workspace.as_ref().and_then(|c| c.terminal.as_ref());
    let user_terminal = user.as_ref().and_then(|c| c.terminal.as_ref());
    let workspace_workspace = workspace.as_ref().and_then(|c| c.workspace.as_ref());
    let user_workspace = user.as_ref().and_then(|c| c.workspace.as_ref());

    let command = workspace_terminal
        .and_then(|t| t.command.clone())
        .or_else(|| user_terminal.and_then(|t| t.command.clone()));
    let args = workspace_terminal
        .and_then(|t| t.args.clone())
        .or_else(|| user_terminal.and_then(|t| t.args.clone()))
        .unwrap_or_default();
    let workspace_root = workspace_workspace
        .and_then(|w| w.root.clone())
        .or_else(|| user_workspace.and_then(|w| w.root.clone()))
        .unwrap_or_else(|| DEFAULT_WORKSPACE_ROOT.to_string());

    Config {
        keymap: KeymapConfig {
            toggle_mode,
            switch_workspace,
            switch_tab,
            refresh,
        },
        terminal: TerminalConfig { command, args },
        workspace: WorkspaceConfig {
            root: PathBuf::from(workspace_root),
        },
    }
}

fn config_path_from_root(root: &Path) -> PathBuf {
    root.join(".blackpepper").join("config.toml")
}

fn user_config_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(config_path_from_root(&home))
}

pub fn load_config(root: &Path) -> Config {
    let workspace_path = config_path_from_root(root);
    let user_path = user_config_path();

    let workspace_config = read_toml(&workspace_path);
    let user_config = user_path.and_then(|path| read_toml(&path));

    merge_config(user_config, workspace_config)
}

#[cfg(test)]
mod tests {
    use super::load_config;
    use std::env;
    use std::fs;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};
    use tempfile::TempDir;

    static HOME_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn home_lock() -> std::sync::MutexGuard<'static, ()> {
        HOME_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    fn write_config(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create config dir");
        }
        fs::write(path, contents).expect("write config");
    }

    #[test]
    fn load_config_uses_defaults_when_empty() {
        let _guard = home_lock();
        let original_home = env::var("HOME").ok();
        let home = TempDir::new().expect("temp home");
        env::set_var("HOME", home.path());

        let repo = TempDir::new().expect("temp repo");
        let config = load_config(repo.path());

        assert_eq!(config.keymap.toggle_mode, "ctrl+g");
        assert_eq!(config.keymap.switch_workspace, "ctrl+p");
        assert_eq!(config.keymap.switch_tab, "ctrl+o");
        assert_eq!(config.keymap.refresh, "ctrl+r");
        assert_eq!(config.terminal.command, None);
        assert!(config.terminal.args.is_empty());
        assert_eq!(config.workspace.root, Path::new(".blackpepper/workspaces"));

        if let Some(home) = original_home {
            env::set_var("HOME", home);
        } else {
            env::remove_var("HOME");
        }
    }

    #[test]
    fn load_config_merges_user_and_workspace() {
        let _guard = home_lock();
        let original_home = env::var("HOME").ok();
        let home = TempDir::new().expect("temp home");
        env::set_var("HOME", home.path());

        let user_config_path = home.path().join(".blackpepper").join("config.toml");
        write_config(
            &user_config_path,
            r#"
[keymap]
toggle_mode = "ctrl+x"
switch_workspace = "ctrl+u"
refresh = "ctrl+z"

[terminal]
command = "zsh"
args = ["-l"]

[workspace]
root = "user/workspaces"
"#,
        );

        let repo = TempDir::new().expect("temp repo");
        let workspace_config_path = repo
            .path()
            .join(".blackpepper")
            .join("config.toml");
        write_config(
            &workspace_config_path,
            r#"
[keymap]
toggle_mode = "ctrl+y"

[terminal]
command = "fish"

[workspace]
root = ".pepper/workspaces"
"#,
        );

        let config = load_config(repo.path());

        assert_eq!(config.keymap.toggle_mode, "ctrl+y");
        assert_eq!(config.keymap.switch_workspace, "ctrl+u");
        assert_eq!(config.keymap.refresh, "ctrl+z");
        assert_eq!(config.terminal.command, Some("fish".to_string()));
        assert_eq!(config.terminal.args, vec!["-l".to_string()]);
        assert_eq!(config.workspace.root, Path::new(".pepper/workspaces"));

        if let Some(home) = original_home {
            env::set_var("HOME", home);
        } else {
            env::remove_var("HOME");
        }
    }
}
