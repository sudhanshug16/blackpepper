use super::{text_path, ClientRuntime};
use crate::client_config::{ClientConfig, StartupCommand};
use crate::core::{SessionRecord, WorkspaceRecord};
use crate::transport::{HostCommand, HostTransport};
use crate::zellij::{PaneProcessState, ZellijRuntime};
use std::path::{Component, Path, PathBuf};

const SERVICE_ID_ENV: &str = "BLACKPEPPER_SERVICE_ID";

impl ClientRuntime {
    pub(super) fn start_configured_services(
        &mut self,
        zellij: &ZellijRuntime,
        session: &SessionRecord,
        workspace: &WorkspaceRecord,
    ) -> Result<(), String> {
        let config = self.workspace_config(workspace)?;
        for service in config.startup.iter().filter(|service| service.auto_start) {
            self.start_service(zellij, session, workspace, &config, service)?;
        }
        Ok(())
    }

    pub(crate) fn start_named_service(
        &mut self,
        workspace_id: crate::core::WorkspaceId,
        name: &str,
    ) -> Result<u64, String> {
        let (lease, workspace) = self.acquire_workspace_session_lease(workspace_id)?;
        let (zellij, session, _) = self.ensure_workspace_session_under_lease(&workspace)?;
        let config = self.workspace_config(&workspace)?;
        let service = config
            .startup
            .iter()
            .find(|service| service.name == name)
            .ok_or_else(|| format!("No configured service is named '{name}'."))?;
        let tab_id = self.start_service(&zellij, &session, &workspace, &config, service)?;
        lease.release()?;
        Ok(tab_id)
    }

    fn start_service(
        &mut self,
        zellij: &ZellijRuntime,
        session: &SessionRecord,
        workspace: &WorkspaceRecord,
        config: &ClientConfig,
        service: &StartupCommand,
    ) -> Result<u64, String> {
        let root = Path::new(&workspace.root_path);
        let candidate = service_cwd(root, service.cwd.as_deref())?;
        let cwd = if workspace.host_id == self.local_host_id {
            canonical_service_cwd(root, &candidate)?
        } else {
            canonical_remote_service_cwd(self.transport_mut(workspace.host_id)?, root, &candidate)?
        };
        let identity = service_identity(workspace, &service.name);
        let marker = format!("{SERVICE_ID_ENV}={identity}");
        let mut arguments = config
            .workspace_env
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>();
        arguments.push(marker.clone());
        arguments.extend(service.command.iter().cloned());
        let command = HostCommand::new("env").args(arguments);
        let tab_name = format!("service-{identity}");

        for attempt in 0..2 {
            let (tab_id, created) = zellij
                .ensure_tab(
                    self.transport_mut(workspace.host_id)?,
                    &session.backend_session_id,
                    &tab_name,
                    &cwd,
                    Some(&command),
                )
                .map_err(|error| error.to_string())?;
            let pane = zellij
                .terminal_pane_for_tab(
                    self.transport_mut(workspace.host_id)?,
                    &session.backend_session_id,
                    tab_id,
                )
                .map_err(|error| error.to_string())?;
            if !pane.has_command_argument(&marker) {
                return Err(format!(
                    "Zellij tab {tab_name} does not belong to configured service '{}'; refusing to report or close it.",
                    service.name
                ));
            }
            if pane.process_state() == PaneProcessState::Live {
                return Ok(tab_id);
            }
            let pane_selector = pane.selector();
            let closed = zellij
                .close_tab_if_pane_matches(
                    self.transport_mut(workspace.host_id)?,
                    &session.backend_session_id,
                    tab_id,
                    &tab_name,
                    &pane_selector,
                    &marker,
                )
                .map_err(|error| format!("Could not close the exited service tab: {error}"))?;
            if !closed {
                return Err(format!(
                    "Configured service '{}' exited, but its tab identity changed before cleanup; it was left open.",
                    service.name
                ));
            }
            if created || attempt == 1 {
                return Err(format!(
                    "Configured service '{}' exited immediately; its tab was closed.",
                    service.name
                ));
            }
        }
        unreachable!("service launch loop returns within two attempts")
    }
}

fn service_identity(workspace: &WorkspaceRecord, name: &str) -> uuid::Uuid {
    uuid::Uuid::new_v5(&workspace.id.as_uuid(), name.as_bytes())
}

fn service_cwd(root: &Path, configured: Option<&Path>) -> Result<PathBuf, String> {
    let Some(configured) = configured else {
        return Ok(root.to_path_buf());
    };
    let cwd = if configured.is_absolute() {
        configured.to_path_buf()
    } else {
        if configured
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
        {
            return Err("Service cwd must stay inside its workspace.".to_string());
        }
        root.join(configured)
    };
    if !cwd.starts_with(root) {
        return Err("Service cwd must stay inside its workspace.".to_string());
    }
    Ok(cwd)
}

fn canonical_service_cwd(root: &Path, candidate: &Path) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(|error| {
        format!(
            "Could not resolve workspace root {}: {error}",
            root.display()
        )
    })?;
    let candidate = candidate.canonicalize().map_err(|error| {
        format!(
            "Could not resolve service cwd {}: {error}",
            candidate.display()
        )
    })?;
    if !candidate.is_dir() || !candidate.starts_with(&root) {
        return Err("Service cwd must resolve to a directory inside its workspace.".to_string());
    }
    Ok(candidate)
}

fn canonical_remote_service_cwd(
    transport: &mut dyn HostTransport,
    root: &Path,
    candidate: &Path,
) -> Result<PathBuf, String> {
    let root = text_path(root)?;
    let candidate = text_path(candidate)?;
    let output = transport
        .exec(&HostCommand::new("sh").args([
            "-c",
            "root=$(realpath -e -- \"$1\") || exit 3; target=$(realpath -e -- \"$2\") || exit 3; test -d \"$target\" || exit 3; case \"$target\" in \"$root\"|\"$root\"/*) printf '%s' \"$target\" ;; *) exit 4 ;; esac",
            "blackpepper-service-cwd",
            &root,
            &candidate,
        ]))
        .map_err(|error| error.to_string())?;
    match output.status {
        Some(0) => {
            let value = String::from_utf8(output.stdout)
                .map_err(|_| "Resolved remote service cwd was not UTF-8.".to_string())?;
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                return Err("Resolved remote service cwd was not absolute.".to_string());
            }
            Ok(path)
        }
        Some(4) => Err("Service cwd must resolve to a directory inside its workspace.".to_string()),
        _ => Err(format!(
            "Could not resolve remote service cwd {}.",
            candidate
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_identity_is_stable_and_does_not_embed_the_label() {
        let workspace = WorkspaceRecord::new(crate::core::HostId::new(), "/srv/app");
        let first = service_identity(&workspace, "api worker / β");
        assert_eq!(first, service_identity(&workspace, "api worker / β"));
        assert_ne!(first, service_identity(&workspace, "another"));
        assert_eq!(format!("service-{first}").len(), 44);
    }

    #[test]
    fn service_cwd_cannot_escape_the_workspace() {
        assert_eq!(
            service_cwd(Path::new("/srv/app"), Some(Path::new("packages/api"))).unwrap(),
            Path::new("/srv/app/packages/api")
        );
        assert!(service_cwd(Path::new("/srv/app"), Some(Path::new("../other"))).is_err());
        assert!(service_cwd(Path::new("/srv/app"), Some(Path::new("/srv/other"))).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn service_cwd_cannot_escape_through_a_symlink() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("linked")).unwrap();

        let candidate = service_cwd(root.path(), Some(Path::new("linked"))).unwrap();
        assert!(canonical_service_cwd(root.path(), &candidate).is_err());
    }
}
