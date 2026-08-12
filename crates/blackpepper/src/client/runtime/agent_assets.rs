use super::ClientRuntime;
use crate::core::HostId;
use crate::providers::runtime::{write_private_atomic, ManagedAsset};
use crate::transport::HostCommand;
use std::io::Write;
use std::path::{Path, PathBuf};

impl ClientRuntime {
    pub(super) fn cleanup_assets_note(
        &mut self,
        host_id: HostId,
        assets: &[ManagedAsset],
    ) -> String {
        if assets.is_empty() {
            return String::new();
        }
        match self.cleanup_assets(host_id, assets) {
            Ok(()) => " Managed run integration files were removed.".to_owned(),
            Err(error) => format!(" Managed run integration cleanup failed: {error}."),
        }
    }

    fn cleanup_assets(&mut self, host_id: HostId, assets: &[ManagedAsset]) -> Result<(), String> {
        if host_id == self.local_host_id {
            return cleanup_local_assets(assets);
        }
        for asset in assets {
            let path = asset
                .path
                .to_str()
                .ok_or_else(|| "Integration cleanup path is not valid UTF-8.".to_owned())?;
            let output = self
                .transport_mut(host_id)?
                .exec(&HostCommand::new("rm").args(["-f", "--", path]))
                .map_err(|error| error.to_string())?;
            if !output.success {
                return Err(format!(
                    "remote cleanup failed for {}",
                    asset.path.display()
                ));
            }
        }
        Ok(())
    }

    pub(super) fn command_path(&mut self, host_id: HostId, name: &str) -> Result<String, String> {
        let output = self
            .transport_mut(host_id)?
            .exec(&HostCommand::new("sh").args([
                "-c",
                "command -v \"$1\" 2>/dev/null || true",
                "blackpepper-provider-lookup",
                name,
            ]))
            .map_err(|error| error.to_string())?;
        let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if path.starts_with('/') {
            Ok(path)
        } else {
            Err(format!(
                "{name} is unavailable on this host. Install and authenticate the provider before spawning it."
            ))
        }
    }

    /// OpenCode has no session-only plugin flag, so the managed plugin uses
    /// its inline-config environment layer. Refuse to replace an existing
    /// value: it may contain user settings or credentials and must never be
    /// read back to the client merely to merge our plugin into it.
    pub(super) fn reserve_opencode_inline_config(&mut self, host_id: HostId) -> Result<(), String> {
        let output = self
            .transport_mut(host_id)?
            .exec(
                &HostCommand::new("sh").args(["-c", "test \"${OPENCODE_CONFIG_CONTENT+x}\" != x"]),
            )
            .map_err(|error| error.to_string())?;
        if output.success {
            Ok(())
        } else {
            Err(
                "OpenCode's OPENCODE_CONFIG_CONTENT environment variable is already set. Blackpepper will not replace or read it. Move that configuration into an OpenCode config file, or unset the variable before starting Blackpepper, then retry."
                    .to_owned(),
            )
        }
    }

    pub(super) fn helper_path(&self, host_id: HostId) -> Result<String, String> {
        if host_id != self.local_host_id {
            return self
                .helper_paths
                .get(&host_id)
                .cloned()
                .ok_or_else(|| "The remote helper is not connected.".to_owned());
        }
        super::helper::sibling_helper_path().map(|path| path.to_string_lossy().into_owned())
    }

    pub(super) fn integration_dir(&mut self, host_id: HostId) -> Result<PathBuf, String> {
        if host_id == self.local_host_id {
            return Ok(self.paths.state_dir().join("integrations"));
        }
        let output = self
            .transport_mut(host_id)?
            .exec(&HostCommand::new("sh").args([
                "-c",
                "printf '%s' \"${XDG_STATE_HOME:-$HOME/.local/state}\"",
            ]))
            .map_err(|error| error.to_string())?;
        let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !root.starts_with('/') {
            return Err("Remote XDG state directory is not absolute.".to_owned());
        }
        Ok(PathBuf::from(root).join("blackpepper/integrations"))
    }

    pub(super) fn install_assets(
        &mut self,
        host_id: HostId,
        assets: &[ManagedAsset],
    ) -> Result<(), String> {
        if host_id == self.local_host_id {
            for asset in assets {
                write_private_atomic(&asset.path, &asset.contents)?;
            }
            return Ok(());
        }
        for asset in assets {
            self.install_remote_asset(host_id, asset)?;
        }
        Ok(())
    }

    fn install_remote_asset(
        &mut self,
        host_id: HostId,
        asset: &ManagedAsset,
    ) -> Result<(), String> {
        let parent = asset
            .path
            .parent()
            .and_then(Path::to_str)
            .ok_or_else(|| "Integration asset has no valid parent directory.".to_owned())?;
        let path = asset
            .path
            .to_str()
            .ok_or_else(|| "Integration path is not valid UTF-8.".to_owned())?;
        let transport = self.transport_mut(host_id)?;
        let prepared = transport
            .exec(&HostCommand::new("install").args(["-d", "-m", "700", parent]))
            .map_err(|error| error.to_string())?;
        if !prepared.success {
            return Err("Could not prepare the remote integration directory.".to_owned());
        }
        let quoted = shell_words::quote(path);
        let script = format!(
            "umask 077; cat > {quoted}.tmp && chmod 600 {quoted}.tmp && mv -f {quoted}.tmp {quoted}"
        );
        let mut child = transport
            .spawn_exec_with_stdin(&HostCommand::new("sh").args(["-c", &script]))
            .map_err(|error| error.to_string())?;
        let mut stdin = child
            .take_stdin()
            .ok_or_else(|| "Remote integration upload has no stdin.".to_owned())?;
        stdin
            .write_all(&asset.contents)
            .map_err(|error| error.to_string())?;
        drop(stdin);
        let output = child
            .wait_with_output()
            .map_err(|error| error.to_string())?;
        if output.success {
            Ok(())
        } else {
            Err("Remote integration upload failed.".to_owned())
        }
    }
}

fn cleanup_local_assets(assets: &[ManagedAsset]) -> Result<(), String> {
    for asset in assets {
        match std::fs::remove_file(&asset.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "could not remove {}: {error}",
                    asset.path.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_asset_cleanup_removes_only_the_exact_run_file() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("claude-run.json");
        let neighbor = directory.path().join("claude-other.json");
        std::fs::write(&target, b"managed").unwrap();
        std::fs::write(&neighbor, b"user").unwrap();
        let asset = ManagedAsset {
            path: target.clone(),
            contents: Vec::new(),
        };

        cleanup_local_assets(std::slice::from_ref(&asset)).unwrap();
        cleanup_local_assets(std::slice::from_ref(&asset)).unwrap();

        assert!(!target.exists());
        assert_eq!(std::fs::read(neighbor).unwrap(), b"user");
    }
}
