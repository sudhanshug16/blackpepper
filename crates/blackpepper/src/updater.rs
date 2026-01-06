//! Auto-update via the installer script.
//!
//! This re-runs the install script on startup (throttled) to fetch the
//! latest release binary. The new version is used on next restart.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const INSTALL_URL: &str =
    "https://raw.githubusercontent.com/sudhanshug16/blackpepper/main/docs/install.sh";
const UPDATE_ENV_DISABLE: &str = "BLACKPEPPER_DISABLE_UPDATE";
const UPDATE_COOLDOWN: Duration = Duration::from_secs(60 * 60 * 24);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOutcome {
    Started,
    Completed,
    SkippedDev,
    SkippedDisabled,
    SkippedCooldown,
    FailedSpawn,
    FailedExit,
}

pub fn apply_staged_update() {
    // The install script replaces the binary directly; no staging needed.
}

pub fn check_for_update() -> UpdateOutcome {
    run_update(false, true, false)
}

#[allow(dead_code)]
pub fn force_update() -> UpdateOutcome {
    run_update(true, true, false)
}

pub fn force_update_sync() -> UpdateOutcome {
    run_update(true, true, true)
}

fn run_update(force: bool, quiet: bool, wait: bool) -> UpdateOutcome {
    if !force && env::var_os(UPDATE_ENV_DISABLE).is_some() {
        return UpdateOutcome::SkippedDisabled;
    }
    let Ok(current_exe) = env::current_exe() else {
        return UpdateOutcome::FailedSpawn;
    };
    if is_dev_binary(&current_exe) {
        return UpdateOutcome::SkippedDev;
    }
    if !force && !should_run_update() {
        return UpdateOutcome::SkippedCooldown;
    }

    let command = format!(
        "if command -v curl >/dev/null 2>&1; then \
            curl -fsSL {INSTALL_URL} | bash; \
        elif command -v wget >/dev/null 2>&1; then \
            wget -qO- {INSTALL_URL} | bash; \
        fi"
    );

    let mut process = Command::new("sh");
    process.arg("-c").arg(command);
    if quiet {
        process.stdout(std::process::Stdio::null());
        process.stderr(std::process::Stdio::null());
    }
    if wait {
        let status = match process.status() {
            Ok(status) => status,
            Err(_) => return UpdateOutcome::FailedSpawn,
        };
        let _ = record_update_attempt();
        if status.success() {
            UpdateOutcome::Completed
        } else {
            UpdateOutcome::FailedExit
        }
    } else {
        if process.spawn().is_err() {
            return UpdateOutcome::FailedSpawn;
        }
        let _ = record_update_attempt();
        UpdateOutcome::Started
    }
}

fn is_dev_binary(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    path_str.contains("/target/debug/") || path_str.contains("/target/release/")
}

fn update_stamp_path() -> Option<PathBuf> {
    let base = dirs::state_dir().or_else(dirs::data_local_dir)?;
    Some(base.join("blackpepper").join("updates").join("last_update"))
}

fn should_run_update() -> bool {
    let Some(path) = update_stamp_path() else {
        return true;
    };
    let Ok(contents) = fs::read_to_string(path) else {
        return true;
    };
    let Ok(last) = contents.trim().parse::<u64>() else {
        return true;
    };
    let last = UNIX_EPOCH + Duration::from_secs(last);
    let Ok(elapsed) = SystemTime::now().duration_since(last) else {
        return true;
    };
    elapsed >= UPDATE_COOLDOWN
}

fn record_update_attempt() -> std::io::Result<()> {
    let Some(path) = update_stamp_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    fs::write(path, now.to_string())
}
