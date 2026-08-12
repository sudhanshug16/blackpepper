use super::super::{connection, ClientRuntime};
use crate::core::{HostId, RequestOperation, ResponsePayload, SessionRecord, WorkspaceRecord};

impl ClientRuntime {
    pub(in crate::client::runtime) fn persist_workspace(
        &mut self,
        workspace: &WorkspaceRecord,
    ) -> Result<(), String> {
        if workspace.host_id != self.local_host_id {
            match connection::registry_operation(
                self,
                workspace.host_id,
                RequestOperation::UpsertWorkspace {
                    workspace: workspace.clone(),
                },
            )? {
                ResponsePayload::Acknowledged => {}
                _ => return Err("bp-host returned an unexpected workspace response.".to_string()),
            }
        }
        self.registry
            .upsert_workspace(workspace)
            .map_err(|error| error.to_string())
    }

    pub(in crate::client::runtime) fn persist_session(
        &mut self,
        host_id: HostId,
        session: &SessionRecord,
    ) -> Result<(), String> {
        if host_id != self.local_host_id {
            match connection::registry_operation(
                self,
                host_id,
                RequestOperation::UpsertSession {
                    session: session.clone(),
                },
            )? {
                ResponsePayload::Acknowledged => {}
                _ => return Err("bp-host returned an unexpected session response.".to_string()),
            }
        }
        self.registry
            .upsert_session(session)
            .map_err(|error| error.to_string())
    }
}
