pub(super) use super::agent_context::AgentRunContext;
use super::agent_context::{AgentContextStore, StoredAgentRunContext};
use crate::agent_status::{
    AgentEventDraft, AgentEventKind, AgentEventSource, AgentEventStore, IntegrationHealth,
    NeedsInputCapability, Provider,
};
use crate::core::{
    AgentProcessObservation, AgentRunBinding, AgentRunId, CorePaths, HostAgentRun, HostAgentUpdate,
    HostRegistry, WorkspaceId,
};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

mod host_run;
mod locking;
mod opencode;
mod query;
mod status;
use host_run::{cleanup_managed_asset, host_run_from};
use locking::lock_mutations;

pub(super) struct HostAgentEvents {
    store: AgentEventStore,
    context: AgentContextStore,
    path: PathBuf,
}

impl HostAgentEvents {
    pub fn open(paths: &CorePaths) -> Result<Self, String> {
        let path = paths.agent_events_path();
        let store = AgentEventStore::open(&path).map_err(|error| error.to_string())?;
        let context = AgentContextStore::open(&path)?;
        Ok(Self {
            store,
            context,
            path,
        })
    }

    pub fn register_run(
        &mut self,
        registry: &HostRegistry,
        context: AgentRunContext,
    ) -> Result<(), String> {
        let _mutation_lock = lock_mutations(&self.path)?;
        self.context.register(registry, context)
    }

    pub fn bind_run(
        &mut self,
        registry: &HostRegistry,
        context: AgentRunContext,
        binding: &AgentRunBinding,
    ) -> Result<(), String> {
        let _mutation_lock = lock_mutations(&self.path)?;
        self.context.bind(registry, context, binding)
    }

    pub fn abort_run(&mut self, context: AgentRunContext) -> Result<(), String> {
        let _mutation_lock = lock_mutations(&self.path)?;
        let stored = self
            .context
            .active(context.run_id)?
            .ok_or_else(|| "Agent run is not registered or is stale.".to_owned())?;
        if stored != context {
            return Err("Agent run abort does not match the registered context.".to_owned());
        }
        self.context.deactivate(context)
    }

    pub fn list_runs(
        &mut self,
        workspace_id: Option<WorkspaceId>,
    ) -> Result<Vec<HostAgentRun>, String> {
        let _mutation_lock = lock_mutations(&self.path)?;
        let records = self.context.active_bound(workspace_id)?;
        let mut runs = Vec::with_capacity(records.len());
        for record in records {
            self.refresh_integration_health_locked(record.context.run_id, now_millis())?;
            let run = self.host_run(&record)?;
            if run.snapshot.state == crate::agent_status::AgentState::Exited {
                // Recover a crash between committing the terminal event and
                // clearing the active descriptor. It must never rediscover as
                // a live conversation.
                let cleanup_error = cleanup_managed_asset(&self.path, &record).err();
                self.context.deactivate(record.context)?;
                if let Some(error) = cleanup_error {
                    return Err(format!(
                        "The exited agent was deactivated, but managed integration cleanup failed: {error}"
                    ));
                }
            } else {
                runs.push(run);
            }
        }
        Ok(runs)
    }

    pub fn reconcile_run(
        &mut self,
        expected: AgentRunContext,
        binding: &AgentRunBinding,
        observation: AgentProcessObservation,
    ) -> Result<HostAgentRun, String> {
        let _mutation_lock = lock_mutations(&self.path)?;
        let record = self
            .context
            .record(expected.run_id)?
            .ok_or_else(|| "Agent run is not registered.".to_owned())?;
        if !record.active {
            return Err("Agent run is stale and cannot be reconciled.".to_owned());
        }
        if record.context != expected || record.binding.as_ref() != Some(binding) {
            return Err("Agent process observation does not match the registered run.".to_owned());
        }
        self.refresh_integration_health_locked(expected.run_id, now_millis())?;
        let existing = self.host_run(&record)?;
        if existing.snapshot.state == crate::agent_status::AgentState::Exited {
            self.context.deactivate(expected)?;
            return Ok(existing);
        }
        match observation {
            AgentProcessObservation::Live => Ok(existing),
            AgentProcessObservation::StateUnknown
                if existing.snapshot.state == crate::agent_status::AgentState::Unknown =>
            {
                Ok(existing)
            }
            AgentProcessObservation::StateUnknown => {
                let update = self.append_supervisor(expected, AgentEventKind::StateUnknown)?;
                Ok(host_run_from(record, update.update.snapshot)?)
            }
            AgentProcessObservation::Missing => self.finish_exited(record, expected, None),
            AgentProcessObservation::Exited { exit_code } => {
                self.finish_exited(record, expected, exit_code)
            }
        }
    }

    /// Commit terminal exit and deactivate discovery before best-effort asset
    /// cleanup. A filesystem permission problem must never keep a dead agent
    /// looking live; the cleanup failure is still returned visibly.
    fn finish_exited(
        &mut self,
        record: StoredAgentRunContext,
        expected: AgentRunContext,
        exit_code: Option<i32>,
    ) -> Result<HostAgentRun, String> {
        let update = self.append_supervisor(expected, AgentEventKind::Exited { exit_code })?;
        self.context.deactivate(expected)?;
        let run = host_run_from(record.clone(), update.update.snapshot)?;
        if let Err(error) = cleanup_managed_asset(&self.path, &record) {
            return Err(format!(
                "The agent exit was persisted and deactivated, but managed integration cleanup failed: {error}"
            ));
        }
        Ok(run)
    }

    #[cfg(test)]
    pub fn append(
        &mut self,
        context: AgentRunContext,
        kind: AgentEventKind,
    ) -> Result<HostAgentUpdate, String> {
        self.append_many(context, std::slice::from_ref(&kind))?
            .into_iter()
            .next()
            .ok_or_else(|| "Single agent event produced no stored update.".to_owned())
    }

    /// Persists one provider hook's ordered semantic reductions under the
    /// same active-run lock and SQLite transaction.
    pub fn append_many(
        &mut self,
        context: AgentRunContext,
        kinds: &[AgentEventKind],
    ) -> Result<Vec<HostAgentUpdate>, String> {
        let _mutation_lock = lock_mutations(&self.path)?;
        self.append_many_locked(
            context,
            kinds,
            AgentEventSource::ProviderIntegration,
            now_millis(),
        )
    }

    fn append_many_locked(
        &mut self,
        context: AgentRunContext,
        kinds: &[AgentEventKind],
        source: AgentEventSource,
        observed_at_ms: u64,
    ) -> Result<Vec<HostAgentUpdate>, String> {
        let stored = self
            .active_context(context.run_id)?
            .ok_or_else(|| "Agent run is not registered or is stale.".to_owned())?;
        if stored != context {
            return Err("Agent event context does not match the registered run.".to_owned());
        }
        if kinds.is_empty() {
            return Ok(Vec::new());
        }
        let drafts = kinds
            .iter()
            .map(|kind| AgentEventDraft {
                host_id: context.host_id,
                workspace_id: context.workspace_id,
                run_id: context.run_id,
                pane_id: context.pane_id,
                provider: context.provider,
                observed_at_ms,
                source,
                kind: *kind,
            })
            .collect::<Vec<_>>();
        let updates = self
            .store
            .append_transient_batch(&drafts, provider_capability(context.provider))
            .map_err(|error| error.to_string())?;
        if updates.iter().any(|update| {
            update.event.host_id != context.host_id
                || update.event.workspace_id != context.workspace_id
                || update.event.pane_id != context.pane_id
        }) {
            return Err("Stored agent event context is inconsistent.".to_owned());
        }
        Ok(updates
            .into_iter()
            .map(|update| HostAgentUpdate {
                host_id: context.host_id,
                workspace_id: context.workspace_id,
                pane_id: context.pane_id,
                update,
            })
            .collect())
    }

    fn active_context(&self, run_id: AgentRunId) -> Result<Option<AgentRunContext>, String> {
        self.context.active(run_id)
    }
}

fn provider_capability(provider: Provider) -> NeedsInputCapability {
    match provider {
        Provider::OpenCode => NeedsInputCapability::ProviderEvents,
        Provider::Codex | Provider::Claude => NeedsInputCapability::ProviderEventsWithOverlay,
    }
}

pub(super) fn healthy_event() -> AgentEventKind {
    AgentEventKind::IntegrationHealthChanged {
        health: IntegrationHealth::Healthy {
            integration_version: Some(1),
        },
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
