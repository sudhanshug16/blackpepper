use super::super::{
    AgentEvent, AgentEventKind, AgentEventSource, BlockerObservation, IntegrationHealth, Provider,
};
use super::{
    AgentStatusTracker, BlockerDisposition, EventDisposition, IgnoredUpdate, StatusAuthority,
};

impl AgentStatusTracker {
    pub fn apply_event(&mut self, event: AgentEvent) -> EventDisposition {
        if event.run_id != self.run_id {
            return EventDisposition::Ignored(IgnoredUpdate::StaleRun);
        }
        if event.provider != self.provider {
            return EventDisposition::Ignored(IgnoredUpdate::ProviderMismatch);
        }
        if self
            .last_event_sequence
            .is_some_and(|sequence| event.sequence <= sequence)
        {
            return EventDisposition::Ignored(IgnoredUpdate::StaleSequence);
        }
        if !valid_source(event.source, event.kind) {
            return EventDisposition::Ignored(IgnoredUpdate::InvalidSource);
        }
        if event.kind == AgentEventKind::NeedsInput
            && !self.needs_input_capability.accepts_provider_events()
        {
            return EventDisposition::Ignored(IgnoredUpdate::CapabilityMismatch);
        }

        self.last_event_sequence = Some(event.sequence);
        self.last_event_at_ms = Some(
            self.last_event_at_ms
                .map_or(event.observed_at_ms, |previous| {
                    previous.max(event.observed_at_ms)
                }),
        );
        self.last_event_kind = Some(event.kind);
        self.authority = match event.source {
            AgentEventSource::ProviderIntegration => StatusAuthority::ProviderIntegration,
            AgentEventSource::IntegrationSupervisor => StatusAuthority::IntegrationSupervisor,
            AgentEventSource::ProcessSupervisor => StatusAuthority::ProcessSupervisor,
        };

        self.apply_event_kind(event.source, event.kind);
        self.revision = self.revision.wrapping_add(1);
        EventDisposition::Applied(self.snapshot())
    }

    pub fn apply_blocker(&mut self, observation: BlockerObservation) -> BlockerDisposition {
        if observation.run_id != self.run_id {
            return BlockerDisposition::Ignored(IgnoredUpdate::StaleRun);
        }
        if !self.needs_input_capability.allows_overlay() {
            return BlockerDisposition::Ignored(IgnoredUpdate::CapabilityMismatch);
        }
        if self
            .last_blocker_sequence
            .is_some_and(|sequence| observation.sequence <= sequence)
        {
            return BlockerDisposition::Ignored(IgnoredUpdate::StaleSequence);
        }
        if self
            .last_event_at_ms
            .is_some_and(|event_at| observation.observed_at_ms < event_at)
        {
            return BlockerDisposition::Ignored(IgnoredUpdate::StaleObservation);
        }
        if observation
            .blocker
            .as_ref()
            .is_some_and(|blocker| blocker.provider != self.provider)
        {
            return BlockerDisposition::Ignored(IgnoredUpdate::ProviderMismatch);
        }

        self.last_blocker_sequence = Some(observation.sequence);
        self.last_blocker_at_ms = Some(observation.observed_at_ms);
        let changed = self.blocker != observation.blocker;
        self.blocker = observation.blocker;
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        BlockerDisposition::Applied {
            snapshot: self.snapshot(),
            changed,
        }
    }

    pub(super) fn apply_event_kind(&mut self, source: AgentEventSource, kind: AgentEventKind) {
        if self.provider != Provider::OpenCode
            && source == AgentEventSource::ProviderIntegration
            && !matches!(kind, AgentEventKind::IntegrationHealthChanged { .. })
            && !self.integration_health.is_healthy()
        {
            self.integration_health = IntegrationHealth::Healthy {
                integration_version: None,
            };
        }

        match kind {
            AgentEventKind::Ready => {
                self.completion_suppressed = false;
                self.set_base_state(super::BaseState::Idle);
            }
            AgentEventKind::Working => {
                self.completion_suppressed = false;
                self.set_base_state(super::BaseState::Working);
            }
            AgentEventKind::NeedsInput => {
                self.completion_suppressed = false;
                self.set_base_state(super::BaseState::NeedsInput);
            }
            AgentEventKind::TurnCompleted => {
                if !self.completion_suppressed {
                    self.set_base_state(super::BaseState::Idle);
                    self.completion_revision = self.completion_revision.wrapping_add(1);
                }
            }
            AgentEventKind::StateUnknown => {
                if source == AgentEventSource::ProcessSupervisor {
                    self.completion_suppressed = true;
                }
                self.set_base_state(super::BaseState::Unknown);
            }
            AgentEventKind::Exited { .. } => {
                self.completion_suppressed = false;
                self.set_base_state(super::BaseState::Exited);
            }
            AgentEventKind::IntegrationHealthChanged { health } => {
                self.integration_health = health;
                if !health.is_healthy() && self.base_state != super::BaseState::Exited {
                    self.base_state = super::BaseState::Unknown;
                }
            }
        }
    }
}

fn valid_source(source: AgentEventSource, kind: AgentEventKind) -> bool {
    match kind {
        AgentEventKind::Exited { .. } => source == AgentEventSource::ProcessSupervisor,
        AgentEventKind::IntegrationHealthChanged { .. } => {
            matches!(
                source,
                AgentEventSource::ProviderIntegration | AgentEventSource::IntegrationSupervisor
            )
        }
        AgentEventKind::StateUnknown => matches!(
            source,
            AgentEventSource::ProviderIntegration | AgentEventSource::ProcessSupervisor
        ),
        AgentEventKind::Ready
        | AgentEventKind::Working
        | AgentEventKind::NeedsInput
        | AgentEventKind::TurnCompleted => source == AgentEventSource::ProviderIntegration,
    }
}
