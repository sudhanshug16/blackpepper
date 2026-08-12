use super::super::DisplayStatus;
use crate::core::{AgentRunId, HostAgentRun, PaneId};

#[derive(Debug, Clone)]
pub struct AgentRunView {
    pub run_id: AgentRunId,
    pub pane_id: PaneId,
    pub tab_id: u64,
    pub provider: crate::agent_status::Provider,
    pub zellij_pane_id: String,
    pub needs_input_capability: String,
    pub snapshot: Option<crate::agent_status::AgentSnapshot>,
    /// Redacted provenance for the latest host-authoritative snapshot.
    pub explain: Option<crate::agent_status::AgentExplain>,
    /// A failed refresh must never leave an old state looking authoritative.
    pub snapshot_error: Option<String>,
    pub seen_completion_revision: u64,
    pub blocker: Option<crate::agent_status::BlockerExplain>,
    pub blocker_watcher_instance: Option<uuid::Uuid>,
    pub blocker_sequence: u64,
    pub blocker_observed_at_ms: Option<u64>,
    /// Sequence visible when this client forwarded ETX to the shared Zellij
    /// session. Codex and Claude cannot distinguish an interrupted turn from a
    /// normal stop, so a later completion event must not manufacture `done`.
    pub interrupted_after_sequence: Option<u64>,
}

impl AgentRunView {
    pub fn from_host_run(run: HostAgentRun) -> Self {
        let needs_input_capability =
            displayed_needs_input_capability(run.provider, Some(&run.snapshot));
        Self {
            run_id: run.run_id,
            pane_id: run.pane_id,
            tab_id: run.binding.tab_id,
            provider: run.provider,
            zellij_pane_id: run.binding.zellij_pane_id,
            needs_input_capability: needs_input_capability.to_owned(),
            snapshot: Some(run.snapshot),
            explain: None,
            snapshot_error: None,
            seen_completion_revision: 0,
            blocker: None,
            blocker_watcher_instance: None,
            blocker_sequence: 0,
            blocker_observed_at_ms: None,
            interrupted_after_sequence: None,
        }
    }

    pub fn display_status(&self) -> DisplayStatus {
        if self.interrupted_after_sequence.is_some() || self.snapshot_error.is_some() {
            return DisplayStatus::Unknown;
        }
        let state = self
            .snapshot
            .as_ref()
            .map(|snapshot| {
                if self.blocker.is_some()
                    && snapshot.state != crate::agent_status::AgentState::Exited
                {
                    crate::agent_status::AgentState::NeedsInput
                } else if snapshot.state == crate::agent_status::AgentState::Done
                    && snapshot.completion_revision <= self.seen_completion_revision
                {
                    crate::agent_status::AgentState::Ready
                } else {
                    snapshot.state
                }
            })
            .unwrap_or_else(|| {
                if self.blocker.is_some() {
                    crate::agent_status::AgentState::NeedsInput
                } else {
                    crate::agent_status::AgentState::Unknown
                }
            });
        DisplayStatus::from_agent(state)
    }

    pub fn mark_interrupted(&mut self) {
        self.interrupted_after_sequence = Some(
            self.snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.last_event_sequence)
                .unwrap_or_default(),
        );
    }

    pub fn displayed_needs_input_capability(&self) -> &str {
        &self.needs_input_capability
    }

    /// Start a fresh host watcher sequence. Sequence numbers are local to one
    /// helper process, so retaining the previous instance's cursor would
    /// silently discard valid transitions after reconnect or watcher restart.
    pub(in crate::client) fn begin_blocker_watcher(&mut self, instance_id: uuid::Uuid) {
        if self.blocker_watcher_instance == Some(instance_id) {
            return;
        }
        self.blocker_watcher_instance = Some(instance_id);
        self.blocker_sequence = 0;
        self.blocker = None;
        self.blocker_observed_at_ms = None;
    }

    /// A locally cached healthy snapshot must not suppress a newer expiry
    /// discovered by the host watcher. Conversely, once a later heartbeat is
    /// visible, queued stale overlays must not reappear.
    pub(in crate::client) fn healthy_snapshot_supersedes_blocker(
        &self,
        observed_at_ms: u64,
    ) -> bool {
        self.provider == crate::agent_status::Provider::OpenCode
            && self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.integration_health.is_healthy()
                    && snapshot
                        .last_event_at_ms
                        .is_none_or(|health_at| health_at >= observed_at_ms)
            })
    }

    pub fn apply_snapshot(&mut self, snapshot: crate::agent_status::AgentSnapshot) {
        let resumed_after_interrupt = self
            .interrupted_after_sequence
            .zip(snapshot.last_event_sequence)
            .is_some_and(|(interrupted_at, observed)| {
                observed > interrupted_at
                    && matches!(
                        snapshot.state,
                        crate::agent_status::AgentState::Working
                            | crate::agent_status::AgentState::NeedsInput
                            | crate::agent_status::AgentState::Ready
                            | crate::agent_status::AgentState::Exited
                    )
            });
        if resumed_after_interrupt {
            self.interrupted_after_sequence = None;
        }
        if snapshot.state == crate::agent_status::AgentState::Exited
            || (self.provider == crate::agent_status::Provider::OpenCode
                && snapshot.integration_health.is_healthy())
        {
            // A healthy OpenCode plugin is the full needs-input authority. A
            // screen overlay captured while it was degraded must not survive
            // plugin recovery, even though new overlay events are ignored.
            self.blocker = None;
            self.blocker_observed_at_ms = None;
        }
        self.needs_input_capability =
            displayed_needs_input_capability(self.provider, Some(&snapshot)).to_owned();
        self.snapshot = Some(snapshot);
        self.snapshot_error = None;
    }

    pub fn apply_host_snapshot(&mut self, snapshot: crate::core::HostAgentSnapshot) {
        self.explain = Some(snapshot.explain);
        self.apply_snapshot(snapshot.snapshot);
    }

    /// Marks the displayed state non-authoritative while retaining the last
    /// snapshot as explicitly stale diagnostic context. Returns true only on
    /// the first occurrence of a distinct failure so polling does not flood
    /// the footer every two seconds.
    pub fn mark_snapshot_error(&mut self, error: String) -> bool {
        let changed = self.snapshot_error.as_deref() != Some(error.as_str());
        self.snapshot_error = Some(error);
        changed
    }
}

fn displayed_needs_input_capability(
    provider: crate::agent_status::Provider,
    snapshot: Option<&crate::agent_status::AgentSnapshot>,
) -> &'static str {
    match provider {
        crate::agent_status::Provider::OpenCode
            if snapshot.is_some_and(|snapshot| snapshot.integration_health.is_healthy()) =>
        {
            "full"
        }
        crate::agent_status::Provider::OpenCode
        | crate::agent_status::Provider::Codex
        | crate::agent_status::Provider::Claude => "partial",
    }
}
