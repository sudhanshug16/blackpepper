use super::super::super::{text_path, ClientRuntime};
use crate::core::HostId;
use crate::transport::{
    install_remote_in_data_home, release_asset_for_version, HostCommand, HttpDownloader,
    ManagedTool, SidecarCache,
};

impl ClientRuntime {
    /// Resolve Blackpepper's branded Zellij only from a checksum-verified
    /// managed archive. A program on PATH can claim the same version string;
    /// accepting it would silently reintroduce the transport bug this build
    /// exists to fix.
    pub(super) fn private_zellij_binary(
        &mut self,
        host_id: HostId,
        expected_version: &str,
    ) -> Result<String, String> {
        let target = self.sidecar_target(host_id)?;
        let asset = release_asset_for_version(ManagedTool::Zellij, expected_version, target)
            .map_err(|error| error.to_string())?;
        if host_id == self.local_host_id {
            let cached = SidecarCache::from_xdg()
                .map_err(|error| error.to_string())?
                .ensure(asset, &HttpDownloader::default())
                .map_err(|error| error.to_string())?;
            let local_binary = text_path(&cached.binary_path)?;
            return self.require_binary_version(host_id, local_binary, expected_version);
        }

        let data_home = self.remote_data_home(host_id)?;
        let remote_directory = data_home
            .join("blackpepper/sidecars/zellij")
            .join(expected_version)
            .join(target.triple());
        let remote_binary = remote_directory.join("zellij");
        let remote_binary = text_path(&remote_binary)?;
        let remote_license = asset
            .license_name
            .map(|name| text_path(&remote_directory.join(name)))
            .transpose()?;
        if let Some(binary_sha256) = asset.binary_sha256 {
            if self.remote_binary_matches_digest(
                host_id,
                &remote_binary,
                binary_sha256,
                remote_license.as_deref(),
                asset.license_sha256,
                expected_version,
            )? {
                return Ok(remote_binary);
            }
        }

        let cached = SidecarCache::from_xdg()
            .map_err(|error| error.to_string())?
            .ensure(asset, &HttpDownloader::default())
            .map_err(|error| error.to_string())?;
        let remote = install_remote_in_data_home(self.transport_mut(host_id)?, &cached, &data_home)
            .map_err(|error| error.to_string())?;
        let binary = text_path(&remote.binary_path)?;
        self.require_binary_version(host_id, binary, expected_version)
    }

    fn remote_binary_matches_digest(
        &mut self,
        host_id: HostId,
        binary: &str,
        expected_sha256: &str,
        license: Option<&str>,
        expected_license_sha256: Option<&str>,
        expected_version: &str,
    ) -> Result<bool, String> {
        if !self.remote_file_matches_digest(host_id, binary, expected_sha256)? {
            return Ok(false);
        }
        match (license, expected_license_sha256) {
            (None, None) => {}
            (Some(path), Some(expected)) => {
                if !self.remote_file_matches_digest(host_id, path, expected)? {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
        self.binary_matches(host_id, binary, expected_version)
    }

    fn remote_file_matches_digest(
        &mut self,
        host_id: HostId,
        path: &str,
        expected_sha256: &str,
    ) -> Result<bool, String> {
        let output = self
            .transport_mut(host_id)?
            .exec(&HostCommand::new("sh").args([
                "-c",
                "test -f \"$1\" || exit 3; exec sha256sum -- \"$1\"",
                "blackpepper-managed-binary",
                path,
            ]))
            .map_err(|error| error.to_string())?;
        if output.status == Some(3) && output.stdout.is_empty() && output.stderr.is_empty() {
            return Ok(false);
        }
        if !output.success {
            return Err(format!("Could not verify managed file at {path}."));
        }
        let actual = String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        Ok(actual == expected_sha256)
    }
}
