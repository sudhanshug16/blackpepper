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

/// The workspace picker. It filters across every host at once, which is the
/// only surface that does — the sidebar stays grouped by host so the two never
/// duplicate each other's job.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspacePicker {
    pub filter: String,
    /// Index into the currently filtered list, not the full workspace list.
    pub selected: usize,
}

/// Which grouped `:help` view is open. Help is a first-class surface rather
/// than a detail blob so unavailable commands can be dimmed in place.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HelpView {
    pub scroll: u16,
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
