use super::super::ClientCommand;
use crate::core::WorkspaceId;

#[derive(Debug, Clone)]
pub struct PendingWorktrunkApproval {
    pub workspace_id: WorkspaceId,
    pub command: ClientCommand,
    pub approval: crate::worktrunk::WorktrunkApprovalToken,
    pub review: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailView {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct PortClickTarget {
    pub workspace_id: WorkspaceId,
    pub target: crate::ports::RemotePortTarget,
    pub x_start: u16,
    pub x_end: u16,
    pub y: u16,
}

impl PortClickTarget {
    pub fn contains(&self, x: u16, y: u16) -> bool {
        self.y == y && x >= self.x_start && x < self.x_end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientMode {
    Work,
    Manage,
    Authenticate,
}
