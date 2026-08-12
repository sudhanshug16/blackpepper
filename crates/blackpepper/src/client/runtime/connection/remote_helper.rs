use super::path_text;
use crate::transport::{
    sha256_file, upload_file_to_child, HostCommand, HostTransport, SidecarTarget, SshTransport,
};
use std::path::{Path, PathBuf};

const HELPER_METADATA_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const HELPER_CLEANUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

struct ManagedHelperLocation {
    target: SidecarTarget,
    directory: PathBuf,
    final_path: PathBuf,
}

pub(super) fn find_helper(transport: &mut SshTransport) -> Result<String, String> {
    find_helper_with(transport, bundled_helper_path)
}

fn find_helper_with(
    transport: &mut dyn HostTransport,
    bundled_helper: impl FnOnce(SidecarTarget) -> Result<PathBuf, String>,
) -> Result<String, String> {
    let lookup = transport
        .exec_timeout(
            &HostCommand::new("sh").args([
                "-c",
                "command -v \"$1\" 2>/dev/null || true",
                "blackpepper-helper-lookup",
                "bp-host",
            ]),
            HELPER_METADATA_TIMEOUT,
        )
        .map_err(|error| error.to_string())?;
    let path = String::from_utf8_lossy(&lookup.stdout).trim().to_string();
    if !path.is_empty() && helper_version_matches(transport, &path)? {
        return Ok(path);
    }

    // Managed helpers are immutable per build and target. Reconnects must
    // probe that exact path before streaming the packaged binary again.
    let managed = managed_helper_location(transport)?;
    let final_path = path_text(&managed.final_path)?;
    if helper_version_matches(transport, &final_path)? {
        return Ok(final_path);
    }

    let local = bundled_helper(managed.target)?;
    install_bundled_helper(transport, &managed, &local)
}

fn helper_version_matches(transport: &mut dyn HostTransport, path: &str) -> Result<bool, String> {
    let output = transport
        .exec_timeout(
            &HostCommand::new(path).arg("--version"),
            HELPER_METADATA_TIMEOUT,
        )
        .map_err(|error| error.to_string())?;
    let version = String::from_utf8_lossy(&output.stdout);
    Ok(output.success && version.split_whitespace().last() == Some(crate::BUILD_ID))
}

fn managed_helper_location(
    transport: &mut dyn HostTransport,
) -> Result<ManagedHelperLocation, String> {
    let environment = transport
        .exec_timeout(
            &HostCommand::new("sh").args([
                "-c",
                "printf '%s\\n%s\\n%s\\n' \"$(uname -s)\" \"$(uname -m)\" \"${XDG_DATA_HOME:-$HOME/.local/share}\"",
            ]),
            HELPER_METADATA_TIMEOUT,
        )
        .map_err(|error| error.to_string())?;
    if !environment.success {
        return Err("Could not identify the remote helper target.".to_string());
    }
    let environment = String::from_utf8_lossy(&environment.stdout);
    let mut values = environment.lines();
    let target = SidecarTarget::from_uname(
        values.next().unwrap_or_default(),
        values.next().unwrap_or_default(),
    )
    .map_err(|error| error.to_string())?;
    if !target.is_linux() {
        return Err("V1 remote workspace hosts must run Linux.".to_string());
    }
    let data_home = PathBuf::from(values.next().unwrap_or_default());
    if !data_home.is_absolute() {
        return Err("Remote XDG data directory is not absolute.".to_string());
    }
    let directory = helper_install_directory(&data_home, crate::BUILD_ID, target.triple());
    let final_path = directory.join("bp-host");
    Ok(ManagedHelperLocation {
        target,
        directory,
        final_path,
    })
}

fn install_bundled_helper(
    transport: &mut dyn HostTransport,
    managed: &ManagedHelperLocation,
    local: &Path,
) -> Result<String, String> {
    let digest = sha256_file(local).map_err(|error| error.to_string())?;
    let temporary = managed
        .directory
        .join(format!(".bp-host-{}.upload", uuid::Uuid::new_v4()));
    let directory_text = path_text(&managed.directory)?;
    let final_text = path_text(&managed.final_path)?;
    let temporary_text = path_text(&temporary)?;
    let prepared = transport
        .exec_timeout(
            &HostCommand::new("install").args(["-d", "-m", "700", &directory_text]),
            HELPER_METADATA_TIMEOUT,
        )
        .map_err(|error| error.to_string())?;
    if !prepared.success {
        return Err("Could not prepare the remote helper directory.".to_string());
    }
    let result = (|| {
        let child = transport
            .spawn_exec_with_stdin(&HostCommand::new("sh").args([
                "-c".to_string(),
                format!("umask 077; cat > {}", shell_words::quote(&temporary_text)),
            ]))
            .map_err(|error| error.to_string())?;
        let uploaded = upload_file_to_child(child, local).map_err(|error| error.to_string())?;
        if !uploaded.success {
            return Err("Remote helper upload failed.".to_string());
        }
        let temporary_quoted = shell_words::quote(&temporary_text);
        let final_quoted = shell_words::quote(&final_text);
        let expected_version = shell_words::quote(crate::BUILD_ID);
        let committed = transport
            .exec_timeout(
                &HostCommand::new("sh").args([
                    "-c".to_string(),
                    format!(
                        "set -eu; actual=$(sha256sum -- {temporary_quoted}); actual=${{actual%% *}}; test \"$actual\" = \"{digest}\"; chmod 700 {temporary_quoted}; version=$({temporary_quoted} --version); set -f; actual_version=; for word in $version; do actual_version=$word; done; test \"$actual_version\" = {expected_version}; mv -f -- {temporary_quoted} {final_quoted}"
                    ),
                ]),
                HELPER_METADATA_TIMEOUT,
            )
            .map_err(|error| error.to_string())?;
        if !committed.success {
            return Err("Remote helper checksum or version verification failed.".to_string());
        }
        Ok(final_text)
    })();

    if result.is_err() {
        // A cancelled restore still gets one bounded, idempotent cleanup
        // attempt. Never replace the install failure with cleanup's result.
        crate::transport::CommandCancellation::mask_current(|| {
            let _ = transport.exec_timeout(
                &HostCommand::new("rm").args(["-f", "--", &temporary_text]),
                HELPER_CLEANUP_TIMEOUT,
            );
        });
    }
    result
}

fn bundled_helper_path(target: SidecarTarget) -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let directory = executable.parent().unwrap_or(Path::new("."));
    let packaged = directory
        .join("sidecars")
        .join(target.triple())
        .join("bp-host");
    if packaged.is_file() {
        return Ok(packaged);
    }
    if SidecarTarget::current().ok() == Some(target) {
        return super::super::helper::sibling_helper_path();
    }
    Err(format!(
        "The release package does not include bp-host for {}.",
        target.triple()
    ))
}

fn helper_install_directory(data_home: &Path, build_id: &str, target: &str) -> PathBuf {
    data_home
        .join("blackpepper/sidecars/bp-host")
        .join(build_id)
        .join(target)
}

#[cfg(test)]
#[path = "remote_helper_stall_tests.rs"]
mod stall_tests;
#[cfg(test)]
#[path = "remote_helper_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "remote_helper_verification_tests.rs"]
mod verification_tests;
