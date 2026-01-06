//! Configuration loading and merging.
//!
//! Config is loaded from two sources with workspace taking precedence:
//! 1. User-level: `~/.config/blackpepper/pepper.toml`
//! 2. Workspace-level: `<repo>/.config/blackpepper/pepper.toml`
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
    pub agent: AgentConfig,
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

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub provider: Option<String>,
    pub command: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    keymap: Option<RawKeymap>,
    terminal: Option<RawTerminal>,
    workspace: Option<RawWorkspace>,
    agent: Option<RawAgent>,
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

#[derive(Debug, Default, Deserialize)]
struct RawAgent {
    provider: Option<String>,
    command: Option<String>,
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
    let workspace_agent = workspace.as_ref().and_then(|c| c.agent.as_ref());
    let user_agent = user.as_ref().and_then(|c| c.agent.as_ref());

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
    let agent_provider = workspace_agent
        .and_then(|agent| agent.provider.clone())
        .or_else(|| user_agent.and_then(|agent| agent.provider.clone()));
    let agent_command = workspace_agent
        .and_then(|agent| agent.command.clone())
        .or_else(|| user_agent.and_then(|agent| agent.command.clone()));

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
        agent: AgentConfig {
            provider: agent_provider,
            command: agent_command,
        },
    }
}

pub fn workspace_config_path(root: &Path) -> PathBuf {
    root.join(".config").join("blackpepper").join("pepper.toml")
}

pub fn user_config_path() -> Option<PathBuf> {
    if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
        if !config_home.trim().is_empty() {
            return Some(
                PathBuf::from(config_home)
                    .join("blackpepper")
                    .join("pepper.toml"),
            );
        }
    }
    let config_root = dirs::config_dir()?;
    Some(config_root.join("blackpepper").join("pepper.toml"))
}

fn legacy_config_path_from_root(root: &Path) -> PathBuf {
    root.join(".blackpepper").join("config.toml")
}

fn legacy_user_config_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(legacy_config_path_from_root(&home))
}

pub fn load_config(root: &Path) -> Config {
    let workspace_path = workspace_config_path(root);
    let user_path = user_config_path();

    let workspace_config =
        read_toml(&workspace_path).or_else(|| read_toml(&legacy_config_path_from_root(root)));
    let user_config = user_path
        .and_then(|path| read_toml(&path))
        .or_else(|| legacy_user_config_path().and_then(|path| read_toml(&path)));

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
mod tests {
    use super::{load_config, save_user_agent_provider, user_config_path};
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
        let original_config_home = env::var("XDG_CONFIG_HOME").ok();
        let home = TempDir::new().expect("temp home");
        let config_home = TempDir::new().expect("temp config");
        env::set_var("HOME", home.path());
        env::set_var("XDG_CONFIG_HOME", config_home.path());

        let repo = TempDir::new().expect("temp repo");
        let config = load_config(repo.path());

        assert_eq!(config.keymap.toggle_mode, "ctrl+g");
        assert_eq!(config.keymap.switch_workspace, "ctrl+p");
        assert_eq!(config.keymap.switch_tab, "ctrl+o");
        assert_eq!(config.keymap.refresh, "ctrl+r");
        assert_eq!(config.terminal.command, None);
        assert!(config.terminal.args.is_empty());
        assert_eq!(config.workspace.root, Path::new(".blackpepper/workspaces"));
        assert!(config.agent.provider.is_none());
        assert!(config.agent.command.is_none());

        if let Some(home) = original_home {
            env::set_var("HOME", home);
        } else {
            env::remove_var("HOME");
        }
        if let Some(config_home) = original_config_home {
            env::set_var("XDG_CONFIG_HOME", config_home);
        } else {
            env::remove_var("XDG_CONFIG_HOME");
        }
    }

    #[test]
    fn load_config_merges_user_and_workspace() {
        let _guard = home_lock();
        let original_home = env::var("HOME").ok();
        let original_config_home = env::var("XDG_CONFIG_HOME").ok();
        let home = TempDir::new().expect("temp home");
        let config_home = TempDir::new().expect("temp config");
        env::set_var("HOME", home.path());
        env::set_var("XDG_CONFIG_HOME", config_home.path());

        let user_config_path = config_home.path().join("blackpepper").join("pepper.toml");
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

[agent]
provider = "codex"
"#,
        );

        let repo = TempDir::new().expect("temp repo");
        let workspace_config_path = repo
            .path()
            .join(".config")
            .join("blackpepper")
            .join("pepper.toml");
        write_config(
            &workspace_config_path,
            r#"
[keymap]
toggle_mode = "ctrl+y"

[terminal]
command = "fish"

[workspace]
root = ".pepper/workspaces"

[agent]
command = "custom pr"
"#,
        );

        let config = load_config(repo.path());

        assert_eq!(config.keymap.toggle_mode, "ctrl+y");
        assert_eq!(config.keymap.switch_workspace, "ctrl+u");
        assert_eq!(config.keymap.refresh, "ctrl+z");
        assert_eq!(config.terminal.command, Some("fish".to_string()));
        assert_eq!(config.terminal.args, vec!["-l".to_string()]);
        assert_eq!(config.workspace.root, Path::new(".pepper/workspaces"));
        assert_eq!(config.agent.provider.as_deref(), Some("codex"));
        assert_eq!(config.agent.command.as_deref(), Some("custom pr"));

        if let Some(home) = original_home {
            env::set_var("HOME", home);
        } else {
            env::remove_var("HOME");
        }
        if let Some(config_home) = original_config_home {
            env::set_var("XDG_CONFIG_HOME", config_home);
        } else {
            env::remove_var("XDG_CONFIG_HOME");
        }
    }

    #[test]
    fn save_user_agent_provider_writes_config() {
        let _guard = home_lock();
        let original_config_home = env::var("XDG_CONFIG_HOME").ok();
        let config_home = TempDir::new().expect("temp config");
        env::set_var("XDG_CONFIG_HOME", config_home.path());

        save_user_agent_provider("codex").expect("save provider");
        let path = user_config_path().expect("config path");
        let contents = fs::read_to_string(&path).expect("read config");
        assert!(contents.contains("[agent]"));
        assert!(contents.contains("provider = \"codex\""));

        if let Some(config_home) = original_config_home {
            env::set_var("XDG_CONFIG_HOME", config_home);
        } else {
            env::remove_var("XDG_CONFIG_HOME");
        }
    }
}
