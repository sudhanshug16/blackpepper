use crate::config::Config;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn publish_bundle(config: &Config, build_id: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&config.bundle_root)
        .map_err(|error| format!("could not create temporary bundle directory: {error}"))?;
    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(&config.bundle_root)
        .map_err(|error| format!("could not stage temporary source bundle: {error}"))?;
    let staged_client = staging.path().join("bp-watch");
    let staged_helper = staging.path().join("bp-host");
    copy_executable(&config.build_output_dir.join("bp"), &staged_client)?;
    strip_executable(config, &staged_client)?;
    copy_executable(&config.build_output_dir.join("bp-host"), &staged_helper)?;
    strip_executable(config, &staged_helper)?;
    verify_version(&staged_client, "blackpepper", build_id)?;
    verify_version(&staged_helper, "bp-host", build_id)?;

    let final_dir = config.bundle_root.join(build_id);
    if final_dir.try_exists().map_err(|error| {
        format!(
            "could not inspect temporary bundle {}: {error}",
            final_dir.display()
        )
    })? {
        return Err(format!(
            "refusing to replace existing temporary bundle: {}",
            final_dir.display()
        ));
    }
    let staging_path = staging.keep();
    std::fs::rename(&staging_path, &final_dir).map_err(|error| {
        format!(
            "could not publish temporary bundle {}: {error}",
            final_dir.display()
        )
    })?;
    Ok(final_dir.join("bp-watch"))
}

pub(crate) fn remove_stopped_client(config: &Config, binary: &Path) -> Result<(), String> {
    let expected_parent = binary.parent().and_then(Path::parent);
    if binary.file_name().and_then(|name| name.to_str()) != Some("bp-watch")
        || expected_parent != Some(config.bundle_root.as_path())
    {
        return Err(format!(
            "refusing to remove unexpected source-run client path: {}",
            binary.display()
        ));
    }
    let metadata = binary.symlink_metadata().map_err(|error| {
        format!(
            "could not inspect stopped source-run client {}: {error}",
            binary.display()
        )
    })?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "refusing to remove non-regular source-run client: {}",
            binary.display()
        ));
    }
    std::fs::remove_file(binary)
        .map_err(|error| format!("could not remove stopped source-run client: {error}"))
}

pub(crate) fn bundle_count(config: &Config) -> Result<usize, String> {
    let entries = std::fs::read_dir(&config.bundle_root)
        .map_err(|error| format!("could not inspect temporary source bundles: {error}"))?;
    let mut count = 0;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("could not inspect temporary source bundle: {error}"))?;
        if entry
            .file_type()
            .map_err(|error| format!("could not inspect temporary source bundle: {error}"))?
            .is_dir()
            && !entry.file_name().to_string_lossy().starts_with(".staging-")
        {
            count += 1;
        }
    }
    Ok(count)
}

fn copy_executable(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = source.symlink_metadata().map_err(|error| {
        format!(
            "temporary build output is unavailable at {}: {error}",
            source.display()
        )
    })?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "temporary build output is not a regular file: {}",
            source.display()
        ));
    }
    std::fs::copy(source, destination).map_err(|error| {
        format!(
            "could not stage temporary build output {}: {error}",
            source.display()
        )
    })?;
    std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o755)).map_err(|error| {
        format!(
            "could not make {} executable: {error}",
            destination.display()
        )
    })
}

fn strip_executable(config: &Config, binary: &Path) -> Result<(), String> {
    let argument = if cfg!(target_os = "macos") {
        "-S"
    } else {
        "--strip-debug"
    };
    let status = Command::new(&config.strip)
        .arg(argument)
        .arg(binary)
        .status()
        .map_err(|error| format!("could not run {}: {error}", config.strip.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} could not compact temporary source bundle ({status})",
            config.strip.display()
        ))
    }
}

fn verify_version(binary: &Path, program: &str, build_id: &str) -> Result<(), String> {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .map_err(|error| format!("could not verify {}: {error}", binary.display()))?;
    let expected = format!("{program} {build_id}\n");
    if output.status.success() && output.stdout == expected.as_bytes() {
        Ok(())
    } else {
        Err(format!(
            "temporary bundle failed exact version verification: {}",
            binary.display()
        ))
    }
}
