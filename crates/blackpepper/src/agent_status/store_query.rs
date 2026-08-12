use rusqlite::{params, OptionalExtension};

use crate::core::AgentRunId;

use super::store_schema::decode_snapshot;
use super::{
    AgentEvent, AgentEventSource, AgentEventStore, AgentEventStoreError, AgentExplain,
    AgentSnapshot, StatusAuthority, StoredAgentUpdate,
};

const MAX_FOLLOW_LIMIT: usize = 1_000;

impl AgentEventStore {
    /// Returns the latest committed snapshot for `run_id`.
    pub fn snapshot(
        &self,
        run_id: AgentRunId,
    ) -> Result<Option<AgentSnapshot>, AgentEventStoreError> {
        let encoded = self
            .connection
            .query_row(
                "SELECT latest_snapshot_json FROM agent_status_runs WHERE run_id = ?1",
                [run_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        encoded
            .map(|value| decode_snapshot(&value, run_id))
            .transpose()
    }

    /// Returns redacted authority diagnostics for the latest committed event.
    /// The event schema contains semantic enums only, never provider payloads.
    pub fn explain(
        &self,
        run_id: AgentRunId,
    ) -> Result<Option<AgentExplain>, AgentEventStoreError> {
        let Some(snapshot) = self.snapshot(run_id)? else {
            return Ok(None);
        };
        let event = snapshot
            .last_event_sequence
            .map(|sequence| {
                self.connection.query_row(
                    "SELECT event_json FROM agent_status_events WHERE run_id = ?1 AND sequence = ?2",
                    params![run_id.to_string(), sequence as i64],
                    |row| row.get::<_, String>(0),
                )
            })
            .transpose()?
            .map(|encoded| serde_json::from_str::<AgentEvent>(&encoded))
            .transpose()?;
        let authority = event
            .as_ref()
            .map(|event| match event.source {
                AgentEventSource::ProviderIntegration => StatusAuthority::ProviderIntegration,
                AgentEventSource::IntegrationSupervisor => StatusAuthority::IntegrationSupervisor,
                AgentEventSource::ProcessSupervisor => StatusAuthority::ProcessSupervisor,
            })
            .unwrap_or(StatusAuthority::None);
        Ok(Some(AgentExplain {
            run_id,
            provider: snapshot.provider,
            state: snapshot.state,
            revision: snapshot.revision,
            authority,
            integration_health: snapshot.integration_health,
            needs_input_capability: snapshot.needs_input_capability,
            completion_revision: snapshot.completion_revision,
            seen_completion_revision: snapshot.seen_completion_revision,
            last_event_sequence: snapshot.last_event_sequence,
            last_event_at_ms: snapshot.last_event_at_ms,
            last_event_kind: event.map(|event| event.kind),
            last_blocker_at_ms: None,
            blocker: None,
        }))
    }

    /// Returns committed updates after `sequence`, ordered for stream replay.
    pub fn follow_after(
        &self,
        run_id: AgentRunId,
        sequence: u64,
        limit: usize,
    ) -> Result<Vec<StoredAgentUpdate>, AgentEventStoreError> {
        if limit == 0 || sequence > i64::MAX as u64 {
            return Ok(Vec::new());
        }
        let mut statement = self.connection.prepare(
            "SELECT sequence, event_json, snapshot_json FROM agent_status_events
             WHERE run_id = ?1 AND sequence > ?2
             ORDER BY sequence ASC LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![
                run_id.to_string(),
                sequence as i64,
                limit.min(MAX_FOLLOW_LIMIT) as i64,
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?;
        let mut updates = Vec::new();
        for row in rows {
            let (stored_sequence, event_json, snapshot_json) = row?;
            let event: AgentEvent = serde_json::from_str(&event_json)?;
            let snapshot = decode_snapshot(&snapshot_json, run_id)?;
            if stored_sequence < 1
                || event.sequence != stored_sequence as u64
                || event.run_id != run_id
                || event.provider != snapshot.provider
                || snapshot.last_event_sequence != Some(event.sequence)
            {
                return Err(AgentEventStoreError::CorruptData(
                    "stored event and snapshot do not describe the same run update",
                ));
            }
            updates.push(StoredAgentUpdate { event, snapshot });
        }
        Ok(updates)
    }
}
