use super::{load, ConfigError};
use crate::test_utils::env_lock;
use std::fs;
use tempfile::TempDir;

#[test]
fn rejects_legacy_tmux_without_changing_file() {
    let _guard = env_lock();
    let repo = TempDir::new().unwrap();
    let config_dir = repo.path().join(".blackpepper");
    fs::create_dir_all(&config_dir).unwrap();
    let path = config_dir.join("config.toml");
    let original = "[tmux.tabs.agent]\ncommand = \"codex\"\n";
    fs::write(&path, original).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", repo.path().join("missing-user"));
    let err = load(repo.path()).unwrap_err();
    assert!(matches!(err, ConfigError::LegacyTmux { .. }));
    assert_eq!(fs::read_to_string(path).unwrap(), original);
    std::env::remove_var("XDG_CONFIG_HOME");
}

#[test]
fn project_startup_and_local_env_merge_strictly() {
    let _guard = env_lock();
    let repo = TempDir::new().unwrap();
    let config_dir = repo.path().join(".blackpepper");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        r#"
[[startup]]
name = "server"
command = ["npm", "run", "dev"]
auto_start = true

[workspace.env]
API_URL = "http://localhost:3000"
"#,
    )
    .unwrap();
    fs::write(
        config_dir.join("config.local.toml"),
        "[workspace.env]\nTOKEN_SOURCE = \"keychain\"\n",
    )
    .unwrap();
    std::env::set_var("XDG_CONFIG_HOME", repo.path().join("missing-user"));
    let config = load(repo.path()).unwrap();
    assert_eq!(config.startup.len(), 1);
    assert!(config.startup[0].auto_start);
    assert_eq!(config.workspace_env.len(), 2);
    std::env::remove_var("XDG_CONFIG_HOME");
}

#[test]
fn unknown_keys_are_actionable_errors() {
    let _guard = env_lock();
    let repo = TempDir::new().unwrap();
    let config_dir = repo.path().join(".blackpepper");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("config.toml"), "magic = true\n").unwrap();
    std::env::set_var("XDG_CONFIG_HOME", repo.path().join("missing-user"));
    let err = load(repo.path()).unwrap_err().to_string();
    assert!(err.contains("unknown field"));
    std::env::remove_var("XDG_CONFIG_HOME");
}

#[test]
fn invalid_workspace_environment_names_are_rejected_before_launch() {
    let _guard = env_lock();
    let repo = TempDir::new().unwrap();
    let config_dir = repo.path().join(".blackpepper");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "[workspace.env]\n'NOT-A-NAME' = 'value'\n",
    )
    .unwrap();
    std::env::set_var("XDG_CONFIG_HOME", repo.path().join("missing-user"));
    let error = load(repo.path()).unwrap_err().to_string();
    assert!(error.contains("valid environment name"));
    std::env::remove_var("XDG_CONFIG_HOME");
}
