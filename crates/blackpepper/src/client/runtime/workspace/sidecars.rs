use super::super::{text_path, ClientRuntime};
use crate::core::HostId;
use crate::transport::{
    install_remote_in_data_home, release_asset, HostCommand, HttpDownloader, ManagedTool,
    SidecarCache, SidecarTarget, TransportError,
};
use crate::providers::runtime::ManagedAsset;
use std::path::PathBuf;

const MANAGED_ZELLIJ_CONFIG: &[u8] =
    include_bytes!("../../../../assets/zellij/config.kdl");

impl ClientRuntime {
    /// Install the immutable, version-scoped appearance used only when the
    /// workspace host has no effective Zellij configuration of its own.
    pub(super) fn managed_zellij_config_path(
        &mut self,
        host_id: HostId,
        version: &str,
    ) -> Result<String, String> {
        let application_data = if host_id == self.local_host_id {
            SidecarCache::from_xdg()
                .map_err(|error| error.to_string())?
                .root()
                .parent()
                .ok_or_else(|| "Blackpepper data directory has no parent.".to_owned())?
                .to_path_buf()
        } else {
            self.remote_data_home(host_id)?.join("blackpepper")
        };
        let path = application_data
            .join("zellij-config")
            .join(version)
            .join("config.kdl");
        self.install_assets(
            host_id,
            &[ManagedAsset {
                path: path.clone(),
                contents: MANAGED_ZELLIJ_CONFIG.to_vec(),
            }],
        )?;
        text_path(&path)
    }

    pub(crate) fn exact_binary(
        &mut self,
        host_id: HostId,
        name: &str,
        expected_version: &str,
    ) -> Result<String, String> {
        let lookup = HostCommand::new("sh").args([
            "-c",
            "command -v \"$1\" 2>/dev/null || true",
            "blackpepper-tool-lookup",
            name,
        ]);
        let output = self
            .transport_mut(host_id)?
            .exec(&lookup)
            .map_err(|error| error.to_string())?;
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() && self.binary_matches(host_id, &path, expected_version)? {
            return Ok(path);
        }
        let tool = match name {
            "zellij" => ManagedTool::Zellij,
            "wt" => ManagedTool::Worktrunk,
            _ => return Err(format!("No managed sidecar is defined for {name}.")),
        };
        let target = self.sidecar_target(host_id)?;
        let remote_data_home = if host_id == self.local_host_id {
            None
        } else {
            Some(self.remote_data_home(host_id)?)
        };
        let managed_root = match &remote_data_home {
            Some(data_home) => data_home.join("blackpepper/sidecars"),
            None => SidecarCache::from_xdg()
                .map_err(|error| error.to_string())?
                .root()
                .to_path_buf(),
        };
        let retained = managed_root
            .join(tool.to_string())
            .join(expected_version)
            .join(target.triple())
            .join(tool.binary_name());
        let retained_text = text_path(&retained)?;
        if self.binary_matches(host_id, &retained_text, expected_version)? {
            return Ok(retained_text);
        }
        if tool.version() != expected_version {
            return Err(format!(
                "Managed {tool} {expected_version} is no longer installed; keep its versioned sidecar until the sessions using it have ended."
            ));
        }
        let asset = release_asset(tool, target).map_err(|error| error.to_string())?;
        let cache = SidecarCache::from_xdg().map_err(|error| error.to_string())?;
        let cached = cache
            .ensure(asset, &HttpDownloader::default())
            .map_err(|error| error.to_string())?;
        if host_id == self.local_host_id {
            return text_path(&cached.binary_path);
        }
        let data_home = remote_data_home.expect("remote hosts resolve a data directory");
        let remote = install_remote_in_data_home(self.transport_mut(host_id)?, &cached, &data_home)
            .map_err(|error| error.to_string())?;
        text_path(&remote.binary_path)
    }

    pub(super) fn binary_matches(
        &mut self,
        host_id: HostId,
        binary: &str,
        expected_version: &str,
    ) -> Result<bool, String> {
        let version = match self
            .transport_mut(host_id)?
            .exec(&HostCommand::new(binary).arg("--version"))
        {
            Ok(version) => version,
            // A local managed binary does not exist before its first download.
            // Treat only that spawn failure as a cache miss; transport and
            // permission failures must remain visible to the user.
            Err(TransportError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error.to_string()),
        };
        let actual = String::from_utf8_lossy(&version.stdout)
            .split_whitespace()
            .last()
            .unwrap_or_default()
            .trim_start_matches('v')
            .to_string();
        Ok(version.success && actual == expected_version)
    }

    fn sidecar_target(&mut self, host_id: HostId) -> Result<SidecarTarget, String> {
        if host_id == self.local_host_id {
            return SidecarTarget::current().map_err(|error| error.to_string());
        }
        let output = self
            .transport_mut(host_id)?
            .exec(
                &HostCommand::new("sh")
                    .args(["-c", "printf '%s\\n%s\\n' \"$(uname -s)\" \"$(uname -m)\""]),
            )
            .map_err(|error| error.to_string())?;
        if !output.success {
            return Err("Could not identify the remote Linux architecture.".to_string());
        }
        let value = String::from_utf8_lossy(&output.stdout);
        let mut lines = value.lines();
        let os = lines.next().unwrap_or_default();
        let architecture = lines.next().unwrap_or_default();
        let target =
            SidecarTarget::from_uname(os, architecture).map_err(|error| error.to_string())?;
        if !target.is_linux() {
            return Err("V1 remote workspace hosts must run Linux.".to_string());
        }
        Ok(target)
    }

    fn remote_data_home(&mut self, host_id: HostId) -> Result<PathBuf, String> {
        let output = self
            .transport_mut(host_id)?
            .exec(
                &HostCommand::new("sh")
                    .args(["-c", "printf '%s' \"${XDG_DATA_HOME:-$HOME/.local/share}\""]),
            )
            .map_err(|error| error.to_string())?;
        let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        if !path.is_absolute() {
            return Err("Remote XDG data directory is not absolute.".to_string());
        }
        Ok(path)
    }
}
