use super::process::run_bounded;
use crate::transport::SidecarTarget;
use std::path::{Path, PathBuf};

pub(super) fn discover_exact_binary(
    label: &str,
    system_binary: &str,
    managed_directory: &str,
    version: &str,
) -> Result<PathBuf, String> {
    let system = validate_exact_binary(PathBuf::from(system_binary), label, version);
    if system.is_ok() {
        return system;
    }
    if let Some(data_dir) = dirs::data_dir() {
        if let Ok(target) = SidecarTarget::current() {
            let managed = data_dir
                .join("blackpepper/sidecars")
                .join(managed_directory)
                .join(version)
                .join(target.triple())
                .join(system_binary);
            if managed.is_file() {
                return validate_exact_binary(managed, label, version);
            }
        }
    }
    system
}

pub(super) fn validate_exact_binary(
    binary: PathBuf,
    label: &str,
    required_version: &str,
) -> Result<PathBuf, String> {
    let output = run_version_with_exec_retry(&binary)
        .map_err(|error| format!("{label} {required_version} is unavailable: {error}"))?;
    let version = String::from_utf8_lossy(&output.stdout);
    let exact = version
        .split_whitespace()
        .last()
        .map(|value| value.trim_start_matches('v'))
        == Some(required_version);
    if !output.status.success() || !exact {
        let found = version.trim();
        let found = if found.is_empty() {
            "an incompatible version"
        } else {
            found
        };
        return Err(format!(
            "{label} {required_version} is required; found {found}."
        ));
    }
    Ok(binary)
}

fn run_version_with_exec_retry(binary: &Path) -> std::io::Result<super::process::BoundedOutput> {
    const TEXT_FILE_BUSY: i32 = 26;
    for attempt in 0..3 {
        match run_bounded(binary.as_os_str(), ["--version"]) {
            Err(error) if error.raw_os_error() == Some(TEXT_FILE_BUSY) && attempt < 2 => {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            result => return result,
        }
    }
    unreachable!("bounded retry loop always returns")
}
