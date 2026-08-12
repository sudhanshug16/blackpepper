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
        Self::new_in_data_home(
            archive,
            local_binary,
            binary_sha256,
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
        let binary_sha256 = binary_sha256.into().to_ascii_lowercase();
        if binary_sha256.len() != 64 || !binary_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(SidecarError::InvalidUploadPlan(
                "sidecar binary checksum must be 64 hexadecimal characters".to_string(),
            ));
        }

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
        Ok(Self {
            asset,
            local_binary: local_binary.into(),
            binary_sha256,
            remote_application_directory,
            remote_directory,
            remote_temporary,
            remote_binary,
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

    pub fn verify_and_commit_command(&self) -> HostCommand {
        let temporary = path_string(&self.remote_temporary);
        let final_path = path_string(&self.remote_binary);
        let temporary = shell_words::quote(&temporary);
        let final_path = shell_words::quote(&final_path);
        let expected = &self.binary_sha256;
        let script = format!(
            "set -eu; actual=$(sha256sum -- {temporary}); actual=${{actual%% *}}; \
             if [ \"$actual\" != \"{expected}\" ]; then \
             echo 'uploaded sidecar checksum mismatch' >&2; exit 74; fi; \
             chmod 700 {temporary}; mv -f -- {temporary} {final_path}"
        );
        HostCommand::new("sh").args(["-c".to_string(), script])
    }

    pub fn cleanup_command(&self) -> HostCommand {
        HostCommand::new("rm").args([
            "-f".to_string(),
            "--".to_string(),
            path_string(&self.remote_temporary),
        ])
    }
}

fn path_string(path: &Path) -> String {
    path.to_str()
        .expect("UploadPlan validates its remote path encoding")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{release_asset, ManagedTool, SidecarTarget};

    #[test]
    fn upload_plan_is_versioned_and_verifies_before_atomic_move() {
        let asset = release_asset(ManagedTool::Zellij, SidecarTarget::LinuxAarch64).unwrap();
        // Constructing the token through verification prevents planning from an
        // unverified download. This test asset uses a matching synthetic digest.
        let synthetic = Box::leak(Box::new(ReleaseAsset {
            trusted_sha256: Some(
                "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881",
            ),
            ..asset.clone()
        }));
        let verified = synthetic.verify(b"x").unwrap();
        let plan = UploadPlan::new(
            verified,
            "/tmp/zellij",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Path::new("/home/dev"),
        )
        .unwrap();

        assert_eq!(
            plan.remote_binary,
            Path::new("/home/dev/.local/share/blackpepper/sidecars/zellij/0.44.3/aarch64-unknown-linux-musl/zellij")
        );
        let command = plan.verify_and_commit_command();
        assert_eq!(command.program, "sh");
        assert!(command.args[1].contains("sha256sum"));
        assert!(command.args[1].contains("exit 74"));
        assert!(command.args[1].contains("mv -f"));

        let custom = UploadPlan::new_in_data_home(
            verified,
            "/tmp/zellij",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Path::new("/srv/blackpepper-data"),
        )
        .unwrap();
        assert_eq!(
            custom.remote_binary,
            Path::new(
                "/srv/blackpepper-data/blackpepper/sidecars/zellij/0.44.3/aarch64-unknown-linux-musl/zellij"
            )
        );
        assert!(custom
            .prepare_command()
            .args
            .contains(&"/srv/blackpepper-data/blackpepper".to_string()));
    }

    #[test]
    fn upload_plan_rejects_macos_asset_for_linux_remote() {
        let asset = release_asset(ManagedTool::Zellij, SidecarTarget::MacOsAarch64).unwrap();
        let synthetic = Box::leak(Box::new(ReleaseAsset {
            trusted_sha256: Some(
                "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881",
            ),
            ..asset.clone()
        }));
        let verified = synthetic.verify(b"x").unwrap();
        assert!(UploadPlan::new(
            verified,
            "/tmp/zellij",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Path::new("/Users/dev"),
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn upload_plan_rejects_non_utf8_remote_paths() {
        use std::os::unix::ffi::OsStrExt;

        let asset = release_asset(ManagedTool::Zellij, SidecarTarget::LinuxAarch64).unwrap();
        let synthetic = Box::leak(Box::new(ReleaseAsset {
            trusted_sha256: Some(
                "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881",
            ),
            ..asset.clone()
        }));
        let verified = synthetic.verify(b"x").unwrap();
        let remote_home = Path::new(std::ffi::OsStr::from_bytes(b"/home/\xff"));
        assert!(UploadPlan::new(
            verified,
            "/tmp/zellij",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            remote_home,
        )
        .is_err());
    }
}
