use std::path::{Path, PathBuf};

use super::sidecar::{sha256_file, ReleaseAsset, SidecarError, VerifiedArchive};
use super::HostCommand;

/// Testable steps for atomically installing a verified helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadPlan {
    pub asset: &'static ReleaseAsset,
    pub local_binary: PathBuf,
    pub binary_sha256: String,
    pub remote_application_directory: PathBuf,
    pub remote_directory: PathBuf,
    pub remote_temporary: PathBuf,
    pub remote_binary: PathBuf,
    pub local_license: Option<PathBuf>,
    pub license_sha256: Option<String>,
    pub remote_license_temporary: Option<PathBuf>,
    pub remote_license: Option<PathBuf>,
}

impl UploadPlan {
    pub fn from_local_binary(
        archive: VerifiedArchive,
        local_binary: impl Into<PathBuf>,
        remote_home: &Path,
    ) -> Result<Self, SidecarError> {
        let local_binary = local_binary.into();
        let binary_sha256 = sha256_file(&local_binary)?;
        Self::new(archive, local_binary, binary_sha256, remote_home)
    }

    pub fn from_local_binary_in_data_home(
        archive: VerifiedArchive,
        local_binary: impl Into<PathBuf>,
        remote_data_home: &Path,
    ) -> Result<Self, SidecarError> {
        let local_binary = local_binary.into();
        let binary_sha256 = sha256_file(&local_binary)?;
        Self::new_in_data_home(archive, local_binary, binary_sha256, remote_data_home)
    }

    pub fn new(
        archive: VerifiedArchive,
        local_binary: impl Into<PathBuf>,
        binary_sha256: impl Into<String>,
        remote_home: &Path,
    ) -> Result<Self, SidecarError> {
        Self::new_with_license(
            archive,
            local_binary,
            binary_sha256,
            None,
            None,
            remote_home,
        )
    }

    pub fn new_with_license(
        archive: VerifiedArchive,
        local_binary: impl Into<PathBuf>,
        binary_sha256: impl Into<String>,
        local_license: Option<&Path>,
        license_sha256: Option<&str>,
        remote_home: &Path,
    ) -> Result<Self, SidecarError> {
        Self::new_with_license_in_data_home(
            archive,
            local_binary,
            binary_sha256,
            local_license,
            license_sha256,
            &remote_home.join(".local/share"),
        )
    }

    /// Plan an install for a host with a custom absolute `XDG_DATA_HOME`.
    pub fn new_in_data_home(
        archive: VerifiedArchive,
        local_binary: impl Into<PathBuf>,
        binary_sha256: impl Into<String>,
        remote_data_home: &Path,
    ) -> Result<Self, SidecarError> {
        Self::new_with_license_in_data_home(
            archive,
            local_binary,
            binary_sha256,
            None,
            None,
            remote_data_home,
        )
    }

    pub fn new_with_license_in_data_home(
        archive: VerifiedArchive,
        local_binary: impl Into<PathBuf>,
        binary_sha256: impl Into<String>,
        local_license: Option<&Path>,
        license_sha256: Option<&str>,
        remote_data_home: &Path,
    ) -> Result<Self, SidecarError> {
        let asset = archive.asset();
        if !asset.target.is_linux() {
            return Err(SidecarError::InvalidUploadPlan(format!(
                "remote sidecars require a Linux asset, got {}",
                asset.target
            )));
        }
        if !remote_data_home.is_absolute() {
            return Err(SidecarError::InvalidUploadPlan(
                "remote XDG data directory must be absolute".to_string(),
            ));
        }
        let Some(remote_data_home_text) = remote_data_home.to_str() else {
            return Err(SidecarError::InvalidUploadPlan(
                "remote XDG data directory must be valid UTF-8 without NUL bytes".to_string(),
            ));
        };
        if remote_data_home_text.contains('\0') {
            return Err(SidecarError::InvalidUploadPlan(
                "remote XDG data directory must be valid UTF-8 without NUL bytes".to_string(),
            ));
        }
        let binary_sha256 = validated_checksum(binary_sha256.into(), "binary")?;
        let license = match (asset.license_name, local_license, license_sha256) {
            (None, None, None) => None,
            (Some(name), Some(path), Some(checksum)) => Some((
                name,
                path.to_path_buf(),
                validated_checksum(checksum.to_owned(), "license")?,
            )),
            _ => {
                return Err(SidecarError::InvalidUploadPlan(
                    "sidecar license path and checksum must match the asset declaration".to_owned(),
                ))
            }
        };

        let remote_application_directory = remote_data_home.join("blackpepper");
        let sidecars_directory = remote_application_directory.join("sidecars");
        let tool_directory = sidecars_directory.join(asset.tool.to_string());
        let version_directory = tool_directory.join(asset.version);
        let remote_directory = version_directory.join(asset.target.triple());
        let remote_binary = remote_directory.join(asset.binary_name);
        let remote_temporary = remote_directory.join(format!(
            ".{}.{}.{}.upload",
            asset.binary_name,
            &binary_sha256[..12],
            uuid::Uuid::new_v4()
        ));
        let (local_license, license_sha256, remote_license_temporary, remote_license) =
            if let Some((name, local, checksum)) = license {
                let remote = remote_directory.join(name);
                let temporary = remote_directory.join(format!(
                    ".{name}.{}.{}.upload",
                    &checksum[..12],
                    uuid::Uuid::new_v4()
                ));
                (Some(local), Some(checksum), Some(temporary), Some(remote))
            } else {
                (None, None, None, None)
            };
        Ok(Self {
            asset,
            local_binary: local_binary.into(),
            binary_sha256,
            remote_application_directory,
            remote_directory,
            remote_temporary,
            remote_binary,
            local_license,
            license_sha256,
            remote_license_temporary,
            remote_license,
        })
    }

    pub fn prepare_command(&self) -> HostCommand {
        let sidecars = self.remote_application_directory.join("sidecars");
        let tool = sidecars.join(self.asset.tool.to_string());
        let version = tool.join(self.asset.version);
        HostCommand::new("install").args([
            "-d".to_string(),
            "-m".to_string(),
            "700".to_string(),
            path_string(&self.remote_application_directory),
            path_string(&sidecars),
            path_string(&tool),
            path_string(&version),
            path_string(&self.remote_directory),
        ])
    }

    /// Stream `local_binary` to this command using `spawn_exec_with_stdin`.
    pub fn receive_command(&self) -> HostCommand {
        let target = path_string(&self.remote_temporary);
        let target = shell_words::quote(&target);
        HostCommand::new("sh").args(["-c".to_string(), format!("umask 077; cat > {target}")])
    }

    pub fn receive_license_command(&self) -> Option<HostCommand> {
        let target = self.remote_license_temporary.as_ref()?;
        let target = path_string(target);
        let target = shell_words::quote(&target);
        Some(HostCommand::new("sh").args(["-c".to_string(), format!("umask 077; cat > {target}")]))
    }

    pub fn verify_and_commit_command(&self) -> HostCommand {
        let temporary = path_string(&self.remote_temporary);
        let final_path = path_string(&self.remote_binary);
        let temporary = shell_words::quote(&temporary);
        let final_path = shell_words::quote(&final_path);
        let expected = &self.binary_sha256;
        let mut script = format!(
            "set -eu; actual=$(sha256sum -- {temporary}); actual=${{actual%% *}}; \
             if [ \"$actual\" != \"{expected}\" ]; then \
             echo 'uploaded sidecar checksum mismatch' >&2; exit 74; fi; "
        );
        if let (Some(license_temporary), Some(license_path), Some(license_expected)) = (
            &self.remote_license_temporary,
            &self.remote_license,
            &self.license_sha256,
        ) {
            let license_temporary = path_string(license_temporary);
            let license_path = path_string(license_path);
            let license_temporary = shell_words::quote(&license_temporary);
            let license_path = shell_words::quote(&license_path);
            script.push_str(&format!(
                "license_actual=$(sha256sum -- {license_temporary}); license_actual=${{license_actual%% *}}; \
                 if [ \"$license_actual\" != \"{license_expected}\" ]; then \
                 echo 'uploaded sidecar license checksum mismatch' >&2; exit 74; fi; \
                 chmod 600 {license_temporary}; mv -f -- {license_temporary} {license_path}; "
            ));
        }
        script.push_str(&format!(
            "chmod 700 {temporary}; mv -f -- {temporary} {final_path}"
        ));
        HostCommand::new("sh").args(["-c".to_string(), script])
    }

    pub fn cleanup_command(&self) -> HostCommand {
        let mut arguments = vec![
            "-f".to_string(),
            "--".to_string(),
            path_string(&self.remote_temporary),
        ];
        if let Some(license) = &self.remote_license_temporary {
            arguments.push(path_string(license));
        }
        HostCommand::new("rm").args(arguments)
    }
}

fn validated_checksum(checksum: String, label: &str) -> Result<String, SidecarError> {
    let checksum = checksum.to_ascii_lowercase();
    if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SidecarError::InvalidUploadPlan(format!(
            "sidecar {label} checksum must be 64 hexadecimal characters"
        )));
    }
    Ok(checksum)
}

fn path_string(path: &Path) -> String {
    path.to_str()
        .expect("UploadPlan validates its remote path encoding")
        .to_string()
}

#[cfg(test)]
#[path = "sidecar_upload_tests.rs"]
mod tests;
