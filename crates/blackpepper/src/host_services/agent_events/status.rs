use super::HostAgentEvents;
use crate::agent_status::{
    AgentEventKind, AgentEventSource, AgentState, IntegrationHealth, Provider,
};
use crate::core::{AgentRunId, HostAgentSnapshot};
use crate::providers::runtime::OPENCODE_HEALTH_STALE_AFTER_MS;

impl HostAgentEvents {
    pub fn snapshot(&mut self, run_id: AgentRunId) -> Result<Option<HostAgentSnapshot>, String> {
        let _mutation_lock = super::lock_mutations(&self.path)?;
        self.refresh_integration_health_locked(run_id, super::now_millis())?;
        self.snapshot_locked(run_id)
    }

    #[cfg(test)]
    pub(in crate::host_services) fn snapshot_at(
        &mut self,
        run_id: AgentRunId,
        observed_at_ms: u64,
    ) -> Result<Option<HostAgentSnapshot>, String> {
        let _mutation_lock = super::lock_mutations(&self.path)?;
        self.refresh_integration_health_locked(run_id, observed_at_ms)?;
        self.snapshot_locked(run_id)
    }

    fn snapshot_locked(&self, run_id: AgentRunId) -> Result<Option<HostAgentSnapshot>, String> {
        let Some(record) = self.context.record(run_id)? else {
            return Ok(None);
        };
        let context = record.context;
        let snapshot = self
            .store
            .snapshot(run_id)
            .map_err(|error| error.to_string())?;
        let explain = self
            .store
            .explain(run_id)
            .map_err(|error| error.to_string())?;
        match (snapshot, explain) {
            (Some(snapshot), Some(explain)) => Ok(Some(HostAgentSnapshot {
                host_id: context.host_id,
                workspace_id: context.workspace_id,
                pane_id: context.pane_id,
                snapshot,
                explain,
            })),
            (None, None) => Ok(None),
            _ => Err("Agent status snapshot and diagnostics were inconsistent.".to_string()),
        }
    }

    /// Refresh and return the compact authority state used by the host-local
    /// blocker watcher. A missing/corrupt freshness read fails closed to stale
    /// at the watcher boundary without exposing provider payloads.
    pub fn integration_health(&mut self, run_id: AgentRunId) -> Result<IntegrationHealth, String> {
        let _mutation_lock = super::lock_mutations(&self.path)?;
        self.refresh_integration_health_locked(run_id, super::now_millis())?;
        Ok(self
            .store
            .snapshot(run_id)
            .map_err(|error| error.to_string())?
            .map_or(IntegrationHealth::Unknown, |snapshot| {
                snapshot.integration_health
            }))
    }

    pub(super) fn refresh_integration_health_locked(
        &mut self,
        run_id: AgentRunId,
        observed_at_ms: u64,
    ) -> Result<(), String> {
        let Some(context) = self.context.active(run_id)? else {
            return Ok(());
        };
        if context.provider != Provider::OpenCode {
            return Ok(());
        }
        let current = self
            .store
            .snapshot(run_id)
            .map_err(|error| error.to_string())?;
        if current
            .as_ref()
            .is_some_and(|snapshot| snapshot.state == AgentState::Exited)
        {
            return Ok(());
        }
        let freshness = self
            .store
            .integration_freshness(run_id)
            .map_err(|error| error.to_string())?;
        if freshness
            .as_ref()
            .is_some_and(|freshness| freshness.provider != context.provider)
        {
            return Err("Integration freshness belongs to another provider.".to_owned());
        }
        let delivery_gap = freshness
            .as_ref()
            .is_some_and(|freshness| freshness.delivery_gap);
        let fresh = freshness.as_ref().is_some_and(|freshness| {
            !freshness.delivery_gap
                && observed_at_ms
                    .checked_sub(freshness.last_seen_at_ms)
                    .is_some_and(|age| age <= OPENCODE_HEALTH_STALE_AFTER_MS)
        });
        let current_health = current
            .as_ref()
            .map_or(IntegrationHealth::Unknown, |snapshot| {
                snapshot.integration_health
            });
        let desired = if fresh {
            IntegrationHealth::Healthy {
                integration_version: freshness.and_then(|value| value.integration_version),
            }
        } else {
            IntegrationHealth::Stale
        };
        if current_health == desired
            || (current.is_none() && freshness.is_none())
            || (!current_health.is_healthy()
                && desired == IntegrationHealth::Stale
                && !delivery_gap
                && current_health != IntegrationHealth::Stale)
        {
            return Ok(());
        }
        self.append_many_locked(
            context,
            &[AgentEventKind::IntegrationHealthChanged { health: desired }],
            AgentEventSource::IntegrationSupervisor,
            observed_at_ms,
        )?;
        Ok(())
    }
}
