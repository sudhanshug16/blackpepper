use super::super::ClientCommand;
use crate::core::{HostId, WorkspaceId};
use ratatui::layout::Rect;

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

/// A Blackpepper-owned action associated with a visible screen region.
///
/// These targets are rebuilt during every render. That keeps hit testing tied
/// to what the person can actually see after responsive layout and scrolling,
/// instead of leaving stale invisible controls behind.
#[derive(Debug, Clone)]
pub enum MouseAction {
    SelectHost(HostId),
    SelectWorkspace(WorkspaceId),
    AttachSelected,
    AttachNext,
    EnterWork,
    EnterManage,
    OpenPicker,
    OpenCommand,
    CloseCommand,
    PrefillCommand(String),
    ChooseCompletion(usize),
    ChoosePicker(WorkspaceId),
    ClosePicker,
    CloseHelp,
    CloseDetail,
    Approve,
    DismissApproval,
    Quit,
    CancelHostOperation,
    ForwardTarget {
        workspace_id: WorkspaceId,
        target: crate::ports::RemotePortTarget,
    },
    ScrollSidebar,
    ScrollPicker,
    ScrollHelp,
    ScrollDetail,
    ScrollApproval,
    ScrollPorts,
}

impl MouseAction {
    pub fn is_scroll_target(&self) -> bool {
        matches!(
            self,
            Self::ScrollSidebar
                | Self::ScrollPicker
                | Self::ScrollHelp
                | Self::ScrollDetail
                | Self::ScrollApproval
                | Self::ScrollPorts
        )
    }
}

#[derive(Debug, Clone)]
pub struct MouseTarget {
    pub area: Rect,
    pub action: MouseAction,
}

impl MouseTarget {
    pub fn contains(&self, x: u16, y: u16) -> bool {
        self.area.width > 0
            && self.area.height > 0
            && x >= self.area.x
            && x < self.area.x.saturating_add(self.area.width)
            && y >= self.area.y
            && y < self.area.y.saturating_add(self.area.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientMode {
    Work,
    Manage,
    Authenticate,
}
