use super::process::run_bounded_timeout;
use crate::transport::{is_blackpepper_zellij_version, local_data_home, SidecarTarget};
use std::path::{Path, PathBuf};

const VERSION_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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

/// Blackpepper's branded Zellij is an owned runtime, not a system dependency.
/// Resolve it only from the private versioned directory populated by the
/// client; a PATH executable with the same version string is not trusted to
/// contain Blackpepper's transport patches.
pub(super) fn discover_zellij_binary(version: &str) -> Result<PathBuf, String> {
    let data_directory = local_data_home().map_err(|error| error.to_string())?;
    discover_zellij_binary_with(Some(&data_directory), "zellij", version)
}

fn discover_zellij_binary_with(
    data_directory: Option<&Path>,
    system_binary: &str,
    version: &str,
) -> Result<PathBuf, String> {
    if !is_blackpepper_zellij_version(version) {
        return discover_exact_binary("Zellij", system_binary, "zellij", version);
    }
    let data_directory = data_directory.ok_or_else(|| {
        format!("Zellij {version} is unavailable: could not resolve the host data directory.")
    })?;
    let target = SidecarTarget::current().map_err(|error| error.to_string())?;
    let managed = data_directory
        .join("blackpepper/sidecars/zellij")
        .join(version)
        .join(target.triple())
        .join("zellij");
    validate_exact_binary(managed, "Zellij", version)
}

pub(super) fn validate_exact_binary(
    binary: PathBuf,
    label: &str,
    required_version: &str,
) -> Result<PathBuf, String> {
    let output = run_version_with_exec_retry(&binary).map_err(|error| {
        format!(
            "{label} {required_version} is unavailable at {}: {error}",
            binary.display()
        )
    })?;
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
        match run_bounded_timeout(binary.as_os_str(), ["--version"], VERSION_PROBE_TIMEOUT) {
            Err(error) if error.raw_os_error() == Some(TEXT_FILE_BUSY) && attempt < 2 => {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            result => return result,
        }
    }
    unreachable!("bounded retry loop always returns")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_discovery_uses_the_supplied_data_root() {
        let root = tempfile::tempdir().unwrap();
        let expected = root
            .path()
            .join("blackpepper/sidecars/zellij")
            .join(crate::transport::PATCHED_ZELLIJ_VERSION)
            .join(SidecarTarget::current().unwrap().triple())
            .join("zellij");

        let error = discover_zellij_binary_with(
            Some(root.path()),
            "zellij",
            crate::transport::PATCHED_ZELLIJ_VERSION,
        )
        .unwrap_err();

        assert!(error.contains(&expected.display().to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn branded_zellij_ignores_an_exact_path_executable() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let system = root.path().join("system-zellij");
        std::fs::write(
            &system,
            format!(
                "#!/bin/sh\nprintf '%s\\n' 'zellij {}'\n",
                crate::transport::PATCHED_ZELLIJ_VERSION
            ),
        )
        .unwrap();
        std::fs::set_permissions(&system, std::fs::Permissions::from_mode(0o700)).unwrap();
        let managed = root
            .path()
            .join("blackpepper/sidecars/zellij")
            .join(crate::transport::PATCHED_ZELLIJ_VERSION)
            .join(SidecarTarget::current().unwrap().triple())
            .join("zellij");
        std::fs::create_dir_all(managed.parent().unwrap()).unwrap();
        std::fs::write(
            &managed,
            format!(
                "#!/bin/sh\nprintf '%s\\n' 'zellij {}'\n",
                crate::transport::PATCHED_ZELLIJ_VERSION
            ),
        )
        .unwrap();
        std::fs::set_permissions(&managed, std::fs::Permissions::from_mode(0o700)).unwrap();

        assert_eq!(
            discover_zellij_binary_with(
                Some(root.path()),
                system.to_str().unwrap(),
                crate::transport::PATCHED_ZELLIJ_VERSION,
            )
            .unwrap(),
            managed
        );
    }
}
