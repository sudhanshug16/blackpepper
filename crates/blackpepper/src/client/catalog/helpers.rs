//! Catalog entry construction and state-derived notes.

use super::{CatalogEntry, CommandGroup};
use crate::client::{ClientState, DisplayStatus};
use crate::ports::ForwardStatus;

pub(super) fn entry(
    group: CommandGroup,
    syntax: &'static str,
    note: String,
    available: bool,
    unavailable_reason: &str,
) -> CatalogEntry {
    CatalogEntry {
        group,
        syntax,
        note: if available {
            note
        } else {
            unavailable_reason.to_owned()
        },
        available,
    }
}

pub(super) fn active_forwards(state: &ClientState) -> usize {
    let workspace = state.selected_workspace.or(state.active_workspace);
    state
        .forwards
        .iter()
        .filter(|forward| {
            Some(forward.workspace_id) == workspace
                && matches!(
                    forward.status,
                    ForwardStatus::Active | ForwardStatus::Direct
                )
        })
        .count()
}

pub(super) fn explain_note(
    state: &ClientState,
    workspace: Option<crate::core::WorkspaceId>,
) -> String {
    let Some(workspace) = workspace else {
        return "agent diagnostics".to_owned();
    };
    match state.statuses.get(&workspace).copied() {
        Some(DisplayStatus::Unknown) => "coverage is partial here".to_owned(),
        _ => "redacted agent evidence".to_owned(),
    }
}
