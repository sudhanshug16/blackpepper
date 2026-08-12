use crate::core::AgentRunId;

use super::status_types::{
    AgentExplain, BaseState, BlockerDisposition, EventDisposition, IgnoredUpdate, StatusAuthority,
};
use super::{
    AgentEventKind, AgentSnapshot, AgentState, BlockerExplain, IntegrationHealth,
    NeedsInputCapability, Provider,
};

mod transitions;

/// Per-client tracker for one pane's current agent run.
///
/// Each client owns its own seen cursor, so viewing a completion on one device
/// does not clear `done` on another device.
#[derive(Clone, Debug)]
pub struct AgentStatusTracker {
    run_id: AgentRunId,
    provider: Provider,
    base_state: BaseState,
    revision: u64,
    completion_revision: u64,
    seen_completion_revision: u64,
    last_event_sequence: Option<u64>,
    last_event_at_ms: Option<u64>,
    last_event_kind: Option<AgentEventKind>,
    authority: StatusAuthority,
    integration_health: IntegrationHealth,
    needs_input_capability: NeedsInputCapability,
    completion_suppressed: bool,
    blocker: Option<BlockerExplain>,
    last_blocker_sequence: Option<u64>,
    last_blocker_at_ms: Option<u64>,
}

impl AgentStatusTracker {
    pub fn new(
        run_id: AgentRunId,
        provider: Provider,
        needs_input_capability: NeedsInputCapability,
    ) -> Self {
        Self {
            run_id,
            provider,
            base_state: BaseState::Unknown,
            revision: 0,
            completion_revision: 0,
            seen_completion_revision: 0,
            last_event_sequence: None,
            last_event_at_ms: None,
            last_event_kind: None,
            authority: StatusAuthority::None,
            integration_health: IntegrationHealth::Unknown,
            needs_input_capability,
            completion_suppressed: false,
            blocker: None,
            last_blocker_sequence: None,
            last_blocker_at_ms: None,
        }
    }

    /// Rehydrates the durable provider state for a transient host-helper
    /// invocation. Screen blockers and client-private seen cursors are not
    /// persisted by the host event store.
    pub fn from_snapshot(snapshot: AgentSnapshot) -> Self {
        let base_state = match snapshot.state {
            AgentState::Unknown => BaseState::Unknown,
            AgentState::Working => BaseState::Working,
            AgentState::NeedsInput => BaseState::NeedsInput,
            AgentState::Done | AgentState::Ready => BaseState::Idle,
            AgentState::Exited => BaseState::Exited,
        };
        Self {
            run_id: snapshot.run_id,
            provider: snapshot.provider,
            base_state,
            revision: snapshot.revision,
            completion_revision: snapshot.completion_revision,
            seen_completion_revision: snapshot.seen_completion_revision,
            last_event_sequence: snapshot.last_event_sequence,
            last_event_at_ms: snapshot.last_event_at_ms,
            last_event_kind: None,
            authority: StatusAuthority::None,
            integration_health: snapshot.integration_health,
            needs_input_capability: snapshot.needs_input_capability,
            completion_suppressed: snapshot.completion_suppressed,
            blocker: None,
            last_blocker_sequence: None,
            last_blocker_at_ms: None,
        }
    }

    pub fn begin_run(
        &mut self,
        run_id: AgentRunId,
        provider: Provider,
        needs_input_capability: NeedsInputCapability,
    ) -> AgentSnapshot {
        self.run_id = run_id;
        self.provider = provider;
        self.base_state = BaseState::Unknown;
        self.revision = self.revision.wrapping_add(1);
        self.completion_revision = 0;
        self.seen_completion_revision = 0;
        self.last_event_sequence = None;
        self.last_event_at_ms = None;
        self.last_event_kind = None;
        self.authority = StatusAuthority::None;
        self.integration_health = IntegrationHealth::Unknown;
        self.needs_input_capability = needs_input_capability;
        self.completion_suppressed = false;
        self.blocker = None;
        self.last_blocker_sequence = None;
        self.last_blocker_at_ms = None;
        self.snapshot()
    }

    pub fn set_needs_input_capability(
        &mut self,
        capability: NeedsInputCapability,
    ) -> AgentSnapshot {
        if self.needs_input_capability != capability {
            self.needs_input_capability = capability;
            if !capability.allows_overlay() {
                self.blocker = None;
            }
            self.revision = self.revision.wrapping_add(1);
        }
        self.snapshot()
    }

    pub fn mark_seen(&mut self) -> AgentSnapshot {
        if self.seen_completion_revision < self.completion_revision {
            self.seen_completion_revision = self.completion_revision;
            self.revision = self.revision.wrapping_add(1);
        }
        self.snapshot()
    }

    pub fn snapshot(&self) -> AgentSnapshot {
        AgentSnapshot {
            run_id: self.run_id,
            provider: self.provider,
            state: self.effective_state(),
            revision: self.revision,
            completion_revision: self.completion_revision,
            seen_completion_revision: self.seen_completion_revision,
            last_event_sequence: self.last_event_sequence,
            last_event_at_ms: self.last_event_at_ms,
            integration_health: self.integration_health,
            needs_input_capability: self.needs_input_capability,
            completion_suppressed: self.completion_suppressed,
        }
    }

    pub fn explain(&self) -> AgentExplain {
        let blocker_is_effective = self.blocker.is_some() && self.base_state != BaseState::Exited;
        AgentExplain {
            run_id: self.run_id,
            provider: self.provider,
            state: self.effective_state(),
            revision: self.revision,
            authority: if blocker_is_effective {
                StatusAuthority::BlockerOverlay
            } else {
                self.authority
            },
            integration_health: self.integration_health,
            needs_input_capability: self.needs_input_capability,
            completion_revision: self.completion_revision,
            seen_completion_revision: self.seen_completion_revision,
            last_event_sequence: self.last_event_sequence,
            last_event_at_ms: self.last_event_at_ms,
            last_event_kind: self.last_event_kind,
            last_blocker_at_ms: self.last_blocker_at_ms,
            blocker: if blocker_is_effective {
                self.blocker.clone()
            } else {
                None
            },
        }
    }

    pub(super) fn set_base_state(&mut self, state: BaseState) {
        self.base_state = state;
        self.blocker = None;
    }

    fn effective_state(&self) -> AgentState {
        if self.blocker.is_some() && self.base_state != BaseState::Exited {
            return AgentState::NeedsInput;
        }
        match self.base_state {
            BaseState::Unknown => AgentState::Unknown,
            BaseState::Working => AgentState::Working,
            BaseState::NeedsInput => AgentState::NeedsInput,
            BaseState::Idle if self.completion_revision > self.seen_completion_revision => {
                AgentState::Done
            }
            BaseState::Idle => AgentState::Ready,
            BaseState::Exited => AgentState::Exited,
        }
    }
}
