use super::{text_path, ClientRuntime};
use crate::client_config::ClientConfig;
use crate::core::{HostId, WorkspaceRecord};
use crate::transport::{HostCommand, HostTransport};
use std::path::{Path, PathBuf};

const MAX_CONFIG_BYTES: usize = 1024 * 1024;

impl ClientRuntime {
    pub(crate) fn restore_host_workspaces(&mut self, host_id: HostId) -> Result<usize, String> {
        self.restore_host_workspaces_with(host_id, || false, |_, _| {})
    }

    pub(super) fn restore_host_workspaces_with(
        &mut self,
        host_id: HostId,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<usize, String> {
        let workspaces = self
            .snapshot()?
            .workspaces
            .into_iter()
            .filter(|workspace| workspace.host_id == host_id)
            .collect::<Vec<_>>();
        restore_each_with(
            &workspaces,
            |workspace| workspace.root_path.clone(),
            |workspace| self.ensure_workspace_session(workspace).map(|_| ()),
            &mut cancelled,
            &mut progress,
        )
    }

    pub(crate) fn restore_workspace(
        &mut self,
        workspace_id: crate::core::WorkspaceId,
    ) -> Result<(), String> {
        let workspace = self
            .registry
            .workspace(workspace_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "The selected workspace no longer exists.".to_string())?;
        self.ensure_workspace_session(&workspace).map(|_| ())
    }

    pub(super) fn workspace_config(
        &mut self,
        workspace: &WorkspaceRecord,
    ) -> Result<ClientConfig, String> {
        let root = Path::new(&workspace.root_path);
        if workspace.host_id == self.local_host_id {
            return crate::client_config::load(root).map_err(|error| error.to_string());
        }
        let config_home = remote_config_home(self.transport_mut(workspace.host_id)?)?;
        let user_path = config_home.join("blackpepper/config.toml");
        let project_path = root.join(".blackpepper/config.toml");
        let local_path = root.join(".blackpepper/config.local.toml");
        let transport = self.transport_mut(workspace.host_id)?;
        let user = read_remote_layer(transport, &user_path)?;
        let project = read_remote_layer(transport, &project_path)?;
        let local = read_remote_layer(transport, &local_path)?;
        crate::client_config::load_contents(user, project, local).map_err(|error| error.to_string())
    }
}

/// Restore entries independently so one stale registration cannot suppress
/// every healthy workspace that follows it.
#[cfg(test)]
fn restore_each<T>(
    entries: &[T],
    label: impl Fn(&T) -> String,
    restore: impl FnMut(&T) -> Result<(), String>,
) -> Result<usize, String> {
    restore_each_with(entries, label, restore, &mut || false, &mut |_, _| {})
}

fn restore_each_with<T>(
    entries: &[T],
    label: impl Fn(&T) -> String,
    mut restore: impl FnMut(&T) -> Result<(), String>,
    cancelled: &mut impl FnMut() -> bool,
    progress: &mut impl FnMut(usize, usize),
) -> Result<usize, String> {
    let mut restored = 0_usize;
    let mut failures = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        if cancelled() {
            return Err(format!(
                "Workspace restoration was cancelled after {restored}/{} registered shell(s).",
                entries.len()
            ));
        }
        progress(index + 1, entries.len());
        match restore(entry) {
            Ok(()) => restored += 1,
            Err(error) => failures.push((label(entry), error)),
        }
    }
    if failures.is_empty() {
        return Ok(restored);
    }

    const SHOWN_FAILURES: usize = 4;
    let mut details = failures
        .iter()
        .take(SHOWN_FAILURES)
        .map(|(label, error)| format!("{label}: {}", bounded_text(error, 320)))
        .collect::<Vec<_>>();
    if failures.len() > SHOWN_FAILURES {
        details.push(format!(
            "{} more unavailable workspace(s)",
            failures.len() - SHOWN_FAILURES
        ));
    }
    Err(format!(
        "Restored {restored}/{} registered workspace shell(s); unavailable: {}",
        entries.len(),
        details.join(" | ")
    ))
}

fn bounded_text(value: &str, maximum_chars: usize) -> String {
    let mut characters = value.chars();
    let bounded = characters.by_ref().take(maximum_chars).collect::<String>();
    if characters.next().is_some() {
        format!("{bounded}…")
    } else {
        bounded
    }
}

fn remote_config_home(transport: &mut dyn HostTransport) -> Result<PathBuf, String> {
    let output = transport
        .exec(
            &HostCommand::new("sh")
                .args(["-c", "printf '%s' \"${XDG_CONFIG_HOME:-$HOME/.config}\""]),
        )
        .map_err(|error| error.to_string())?;
    if !output.success {
        return Err("Could not locate the remote XDG config directory.".to_string());
    }
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if !path.is_absolute() {
        return Err("Remote XDG config directory is not absolute.".to_string());
    }
    Ok(path)
}

fn read_remote_layer(
    transport: &mut dyn HostTransport,
    path: &Path,
) -> Result<Option<(PathBuf, String)>, String> {
    let path_text = text_path(path)?;
    let output = transport
        .exec(&HostCommand::new("sh").args([
            "-c",
            "test -f \"$1\" || exit 3; exec cat -- \"$1\"",
            "blackpepper-config",
            &path_text,
        ]))
        .map_err(|error| error.to_string())?;
    if output.status == Some(3) {
        return Ok(None);
    }
    if !output.success {
        return Err(format!("Could not read remote config {}.", path.display()));
    }
    if output.stdout.len() > MAX_CONFIG_BYTES {
        return Err(format!(
            "Remote config {} exceeds {MAX_CONFIG_BYTES} bytes.",
            path.display()
        ));
    }
    let contents = String::from_utf8(output.stdout)
        .map_err(|_| format!("Remote config {} is not UTF-8.", path.display()))?;
    Ok(Some((path.to_path_buf(), contents)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restoration_continues_after_each_unavailable_workspace() {
        let entries = ["stale", "healthy", "bad-config"];
        let mut attempted = Vec::new();

        let error = restore_each(
            &entries,
            |entry| (*entry).to_owned(),
            |entry| {
                attempted.push(*entry);
                (*entry == "healthy")
                    .then_some(())
                    .ok_or_else(|| format!("{entry} unavailable"))
            },
        )
        .unwrap_err();

        assert_eq!(attempted, entries);
        assert!(error.contains("Restored 1/3"));
        assert!(error.contains("stale unavailable"));
        assert!(error.contains("bad-config unavailable"));
    }
}
