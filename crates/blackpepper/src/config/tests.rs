use super::{load_config, save_user_agent_provider, user_config_path, TmuxTabConfig};
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

    assert_eq!(config.keymap.toggle_mode, "ctrl+]");
    assert_eq!(config.keymap.switch_workspace, "ctrl+p");
    assert_eq!(config.tmux.command.as_deref(), Some("tmux"));
    assert!(config.tmux.args.is_empty());
    assert!(config.tmux.tabs.is_empty());
    assert_eq!(config.workspace.root, Path::new(".blackpepper/workspaces"));
    assert!(config.workspace.setup_scripts.is_empty());
    assert!(config.agent.provider.is_none());
    assert!(config.agent.command.is_none());
    assert_eq!(config.upstream.provider, "github");
    assert_eq!(config.ui.background, (0x33, 0x33, 0x33));
    assert_eq!(config.ui.foreground, (0xff, 0xff, 0xff));

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

    let user_config_path = config_home.path().join("blackpepper").join("config.toml");
    write_config(
        &user_config_path,
        r##"
[keymap]
toggle_mode = "ctrl+x"
switch_workspace = "ctrl+u"

[tmux]
command = "tmux"
args = ["-f", "/tmp/tmux.conf"]

[tmux.tabs.user]
command = "echo user"

[workspace]
root = "user/workspaces"

[workspace.setup]
scripts = ["user-setup"]

[agent]
provider = "codex"

[upstream]
provider = "gitlab"

[ui]
background = "#111111"
foreground = "#eeeeee"
"##,
    );

    let repo = TempDir::new().expect("temp repo");
    let workspace_config_path = repo
        .path()
        .join(".config")
        .join("blackpepper")
        .join("config.toml");
    write_config(
        &workspace_config_path,
        r##"
[keymap]
toggle_mode = "ctrl+y"

[tmux]
command = "tmux"
args = ["-L", "alt"]

[tmux.tabs.workspace]
command = "make server"

[tmux.tabs.logs]

[workspace]
root = ".pepper/workspaces"

[workspace.setup]
scripts = ["workspace-setup"]

[agent]
command = "custom pr"

[upstream]
provider = "github"

[ui]
foreground = "#cccccc"
"##,
    );

    let config = load_config(repo.path());

    assert_eq!(config.keymap.toggle_mode, "ctrl+y");
    assert_eq!(config.keymap.switch_workspace, "ctrl+u");
    assert_eq!(config.tmux.command.as_deref(), Some("tmux"));
    assert_eq!(config.tmux.args, vec!["-L".to_string(), "alt".to_string()]);
    assert_eq!(
        config.tmux.tabs,
        vec![
            TmuxTabConfig {
                name: "logs".to_string(),
                command: None,
            },
            TmuxTabConfig {
                name: "workspace".to_string(),
                command: Some("make server".to_string()),
            },
        ]
    );
    assert_eq!(config.workspace.root, Path::new(".pepper/workspaces"));
    assert_eq!(
        config.workspace.setup_scripts,
        vec!["workspace-setup".to_string()]
    );
    assert_eq!(config.agent.provider.as_deref(), Some("codex"));
    assert_eq!(config.agent.command.as_deref(), Some("custom pr"));
    assert_eq!(config.upstream.provider, "github");
    assert_eq!(config.ui.background, (0x11, 0x11, 0x11));
    assert_eq!(config.ui.foreground, (0xcc, 0xcc, 0xcc));

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
