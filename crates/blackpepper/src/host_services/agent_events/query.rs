use super::{host_run_from, provider_capability, AgentRunContext, HostAgentEvents};
use crate::agent_status::{AgentEventDraft, AgentEventKind, AgentEventSource, AgentStatusTracker};
use crate::core::{AgentRunId, HostAgentRun, HostAgentUpdate};

impl HostAgentEvents {
    pub fn follow(
        &self,
        run_id: AgentRunId,
        after_sequence: u64,
        limit: usize,
    ) -> Result<Vec<HostAgentUpdate>, String> {
        let Some(record) = self.context.record(run_id)? else {
            return Ok(Vec::new());
        };
        let context = record.context;
        let updates = self
            .store
            .follow_after(run_id, after_sequence, limit)
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

    pub fn context(&self, run_id: AgentRunId) -> Result<Option<AgentRunContext>, String> {
        self.context.active(run_id)
    }

    pub(super) fn append_supervisor(
        &mut self,
        context: AgentRunContext,
        kind: AgentEventKind,
    ) -> Result<HostAgentUpdate, String> {
        let draft = AgentEventDraft {
            host_id: context.host_id,
            workspace_id: context.workspace_id,
            run_id: context.run_id,
            pane_id: context.pane_id,
            provider: context.provider,
            observed_at_ms: super::now_millis(),
            source: AgentEventSource::ProcessSupervisor,
            kind,
        };
        let update = self
            .store
            .append_transient(draft, provider_capability(context.provider))
            .map_err(|error| error.to_string())?;
        Ok(HostAgentUpdate {
            host_id: context.host_id,
            workspace_id: context.workspace_id,
            pane_id: context.pane_id,
            update,
        })
    }

    pub(super) fn host_run(
        &self,
        record: &super::StoredAgentRunContext,
    ) -> Result<HostAgentRun, String> {
        let snapshot = self
            .store
            .snapshot(record.context.run_id)
            .map_err(|error| error.to_string())?
            .unwrap_or_else(|| {
                AgentStatusTracker::new(
                    record.context.run_id,
                    record.context.provider,
                    provider_capability(record.context.provider),
                )
                .snapshot()
            });
        host_run_from(record.clone(), snapshot)
    }
}
