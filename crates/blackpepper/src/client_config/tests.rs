use super::{load, load_contents, ColorTier, ConfigError};
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

#[test]
fn v2_palette_is_the_default_and_custom_colors_still_override_it() {
    let defaults = load_contents(None, None, None).unwrap();
    assert_eq!(defaults.ui.background, (0x1c, 0x1d, 0x1f));
    assert_eq!(defaults.ui.foreground, (0xe6, 0xe4, 0xe1));

    let custom = load_contents(
        Some((
            "user.toml".into(),
            "[ui]\nbackground = '#112233'\nforeground = '#ddeeff'\n".to_owned(),
        )),
        None,
        None,
    )
    .unwrap();
    assert_eq!(custom.ui.background, (0x11, 0x22, 0x33));
    assert_eq!(custom.ui.foreground, (0xdd, 0xee, 0xff));
}

#[test]
fn terminal_color_tiers_have_deterministic_precedence() {
    let _guard = env_lock();
    let previous = ["NO_COLOR", "COLORTERM", "TERM"].map(std::env::var_os);

    std::env::remove_var("NO_COLOR");
    std::env::set_var("COLORTERM", "truecolor");
    std::env::set_var("TERM", "xterm-256color");
    assert_eq!(
        load_contents(None, None, None).unwrap().ui.color_tier,
        ColorTier::TrueColor
    );

    std::env::set_var("COLORTERM", "");
    assert_eq!(
        load_contents(None, None, None).unwrap().ui.color_tier,
        ColorTier::Ansi256
    );

    std::env::set_var("TERM", "xterm");
    assert_eq!(
        load_contents(None, None, None).unwrap().ui.color_tier,
        ColorTier::Ansi16
    );

    std::env::set_var("NO_COLOR", "1");
    std::env::set_var("COLORTERM", "24bit");
    assert_eq!(
        load_contents(None, None, None).unwrap().ui.color_tier,
        ColorTier::NoColor
    );

    for (name, value) in ["NO_COLOR", "COLORTERM", "TERM"].into_iter().zip(previous) {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}
