use super::{canonical_local, connection, text_path, ClientRuntime};
use crate::core::{
    GroupingPolicy, HostId, RequestOperation, ResponsePayload, WorkspaceId, WorkspaceRecord,
};
use crate::transport::{HostCommand, HostTransport};
use crate::workspace_identity::detect_local;
use std::path::{Path, PathBuf};

mod persistence;
mod session;
mod sidecars;

impl ClientRuntime {
    pub(crate) fn register_workspace(
        &mut self,
        host_id: HostId,
        path: &Path,
    ) -> Result<WorkspaceId, String> {
        let local = host_id == self.local_host_id;
        let root = if local {
            canonical_local(path)?
        } else {
            canonical_remote(self.transport_mut(host_id)?, path)?
        };
        let root_text = text_path(&root)?;
        if !local {
            let payload = connection::registry_operation(
                self,
                host_id,
                RequestOperation::RegisterWorkspace {
                    root_path: root_text,
                    display_name: None,
                },
            )?;
            let ResponsePayload::HostService { payload } = payload else {
                return Err(
                    "bp-host returned an unexpected workspace registration response.".to_string(),
                );
            };
            let crate::core::HostServicePayload::WorkspaceRegistered { workspace } = *payload
            else {
                return Err(
                    "bp-host returned an unexpected workspace registration response.".to_string(),
                );
            };
            if workspace.host_id != host_id {
                return Err("bp-host returned a workspace for a different host.".to_string());
            }
            self.registry
                .upsert_workspace(&workspace)
                .map_err(|error| error.to_string())?;
            return Ok(workspace.id);
        }
        if let Some(existing) = self
            .snapshot()?
            .workspaces
            .into_iter()
            .find(|workspace| workspace.host_id == host_id && workspace.root_path == root_text)
        {
            return Ok(existing.id);
        }

        let mut workspace = WorkspaceRecord::new(host_id, root_text);
        workspace.repository = detect_local(&root, host_id)?.map(|detected| detected.identity);
        self.persist_workspace(&workspace)?;
        Ok(workspace.id)
    }

    pub(crate) fn find_workspace(&self, selector: &str) -> Result<WorkspaceRecord, String> {
        let mut matches = self.snapshot()?.workspaces.into_iter().filter(|workspace| {
            workspace.id.to_string() == selector
                || workspace.id.to_string().starts_with(selector)
                || workspace.display_name.as_deref() == Some(selector)
                || Path::new(&workspace.root_path)
                    .file_name()
                    .is_some_and(|name| name == selector)
        });
        let workspace = matches
            .next()
            .ok_or_else(|| format!("No workspace matches '{selector}'."))?;
        if matches.next().is_some() {
            return Err(format!("Workspace selector '{selector}' is ambiguous."));
        }
        Ok(workspace)
    }

    pub(crate) fn ungroup_workspace(
        &mut self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceRecord, String> {
        let mut workspace = self
            .registry
            .workspace(workspace_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "The selected workspace no longer exists.".to_string())?;
        workspace.grouping = GroupingPolicy::Ungrouped;
        workspace.touch();
        self.persist_workspace(&workspace)?;
        Ok(workspace)
    }
}

fn canonical_remote(transport: &mut dyn HostTransport, path: &Path) -> Result<PathBuf, String> {
    let output = transport
        .exec(&HostCommand::new("sh").args([
            "-c".to_string(),
            "test -d \"$1\" && realpath \"$1\"".to_string(),
            "blackpepper-workspace".to_string(),
            text_path(path)?,
        ]))
        .map_err(|error| error.to_string())?;
    if !output.success {
        return Err(format!(
            "Remote workspace does not exist: {}",
            path.display()
        ));
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !value.starts_with('/') {
        return Err("Remote realpath did not return an absolute path.".to_string());
    }
    Ok(PathBuf::from(value))
}

fn provisional_attachment_count(clients_before: usize) -> usize {
    clients_before.saturating_add(1)
}

#[cfg(test)]
mod tests;
