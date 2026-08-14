mod managed;

use super::super::{text_path, ClientRuntime};
use crate::core::HostId;
use crate::providers::runtime::ManagedAsset;
use crate::transport::{
    install_remote_in_data_home, is_blackpepper_zellij_version, release_asset_for_version,
    sha256_bytes, HostCommand, HttpDownloader, ManagedTool, SidecarCache, SidecarTarget,
    TransportError,
};
use std::path::PathBuf;
use std::time::Duration;

/// A host's own Zellij configuration is read before merging. The cap is
/// generous for a config file and small enough that a wrong path cannot pull
/// something huge across an SSH link.
const MAX_HOST_CONFIG_BYTES: usize = 512 * 1024;
const BINARY_VERSION_TIMEOUT: Duration = Duration::from_secs(5);

impl ClientRuntime {
    /// Install the configuration Zellij is launched with: the host's own file
    /// plus every appearance setting it does not already define.
    ///
    /// Zellij autogenerates a config on first run, so "the host has no config"
    /// is almost never true and cannot be the condition for styling anything.
    /// Merging per setting means a host that only ever set keybindings keeps
    /// them and still gets the appearance, while any opinion it did express
    /// wins. The result is written to a Blackpepper-owned path — the host's
    /// own file is never modified.
    ///
    /// The path carries the merged content's hash, so production and
    /// development clients cannot rewrite a file an existing Zellij session is
    /// watching, and identical content is shared rather than rewritten.
    pub(super) fn managed_zellij_config_path(
        &mut self,
        host_id: HostId,
        version: &str,
        host_config: Option<&str>,
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
        let merged = crate::zellij::appearance::merge(
            host_config.unwrap_or_default(),
            crate::zellij::appearance::APPEARANCE,
        );
        let contents = merged.into_bytes();
        let path = application_data
            .join("zellij-config")
            .join(version)
            .join(sha256_bytes(&contents))
            .join("config.kdl");
        self.install_assets(
            host_id,
            &[ManagedAsset {
                path: path.clone(),
                contents: contents.clone(),
            }],
        )?;
        text_path(&path)
    }

    /// Read the host's own Zellij configuration so it can be merged. A missing
    /// file is not an error: it just means Blackpepper contributes everything.
    pub(super) fn read_host_zellij_config(
        &mut self,
        host_id: HostId,
        path: &str,
    ) -> Result<Option<String>, String> {
        let output = self
            .transport_mut(host_id)?
            .exec(&HostCommand::new("sh").args([
                "-c",
                "test -f \"$1\" || exit 3; exec cat -- \"$1\"",
                "blackpepper-zellij-config",
                path,
            ]))
            .map_err(|error| error.to_string())?;
        if output.status == Some(3) {
            return Ok(None);
        }
        if !output.success {
            return Err(format!("Could not read Zellij configuration at {path}."));
        }
        if output.stdout.len() > MAX_HOST_CONFIG_BYTES {
            return Err(format!(
                "Zellij configuration at {path} exceeds {MAX_HOST_CONFIG_BYTES} bytes."
            ));
        }
        String::from_utf8(output.stdout)
            .map(Some)
            .map_err(|_| format!("Zellij configuration at {path} is not UTF-8."))
    }

    pub(crate) fn exact_binary(
        &mut self,
        host_id: HostId,
        name: &str,
        expected_version: &str,
    ) -> Result<String, String> {
        let tool = match name {
            "zellij" => ManagedTool::Zellij,
            "wt" => ManagedTool::Worktrunk,
            _ => return Err(format!("No managed executable is defined for {name}.")),
        };
        if tool == ManagedTool::Zellij && is_blackpepper_zellij_version(expected_version) {
            return self.private_zellij_binary(host_id, expected_version);
        }

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
        let asset = release_asset_for_version(tool, expected_version, target)
            .map_err(|_| format!(
                "Managed {tool} {expected_version} is unavailable; its versioned executable is required until the sessions using it have ended."
            ))?;
        let cache = SidecarCache::from_xdg().map_err(|error| error.to_string())?;
        let cached = cache
            .ensure(asset, &HttpDownloader::default())
            .map_err(|error| error.to_string())?;
        if host_id == self.local_host_id {
            let binary = text_path(&cached.binary_path)?;
            return self.require_binary_version(host_id, binary, expected_version);
        }
        let data_home = remote_data_home.expect("remote hosts resolve a data directory");
        let remote = install_remote_in_data_home(self.transport_mut(host_id)?, &cached, &data_home)
            .map_err(|error| error.to_string())?;
        let binary = text_path(&remote.binary_path)?;
        self.require_binary_version(host_id, binary, expected_version)
    }

    fn require_binary_version(
        &mut self,
        host_id: HostId,
        binary: String,
        expected_version: &str,
    ) -> Result<String, String> {
        if self.binary_matches(host_id, &binary, expected_version)? {
            Ok(binary)
        } else {
            Err(format!(
                "The managed executable at {binary} does not report required version {expected_version}."
            ))
        }
    }

    pub(super) fn binary_matches(
        &mut self,
        host_id: HostId,
        binary: &str,
        expected_version: &str,
    ) -> Result<bool, String> {
        let version = match self.transport_mut(host_id)?.exec_timeout(
            &HostCommand::new(binary).arg("--version"),
            BINARY_VERSION_TIMEOUT,
        ) {
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
