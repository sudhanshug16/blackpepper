use rusqlite::{params, TransactionBehavior};

use super::store_fs::secure_sqlite_files;
use super::store_schema::{load_run, validate_stored_snapshot};
use super::{
    AgentEvent, AgentEventDraft, AgentEventStore, AgentEventStoreError, AgentStatusTracker,
    EventDisposition, StoredAgentUpdate,
};

impl AgentEventStore {
    /// Allocates the event sequence, applies it, and commits event + snapshot.
    pub fn append(
        &mut self,
        tracker: &mut AgentStatusTracker,
        draft: AgentEventDraft,
    ) -> Result<StoredAgentUpdate, AgentEventStoreError> {
        self.append_batch(tracker, std::slice::from_ref(&draft))?
            .into_iter()
            .next()
            .ok_or(AgentEventStoreError::CorruptData(
                "single-event batch returned no update",
            ))
    }

    /// Applies an ordered group of already-reduced events in one SQLite
    /// transaction. This is used for lifecycle hooks that establish both
    /// integration health and the provider's initial prompt state.
    pub fn append_batch(
        &mut self,
        tracker: &mut AgentStatusTracker,
        drafts: &[AgentEventDraft],
    ) -> Result<Vec<StoredAgentUpdate>, AgentEventStoreError> {
        let Some(first) = drafts.first() else {
            return Ok(Vec::new());
        };
        let current = tracker.snapshot();
        for draft in drafts {
            if draft.run_id != current.run_id {
                return Err(AgentEventStoreError::CrossRun {
                    expected: current.run_id,
                    received: draft.run_id,
                });
            }
            if draft.provider != current.provider {
                return Err(AgentEventStoreError::ProviderMismatch {
                    expected: current.provider,
                    received: draft.provider,
                });
            }
            if draft.host_id != first.host_id
                || draft.workspace_id != first.workspace_id
                || draft.pane_id != first.pane_id
            {
                return Err(AgentEventStoreError::ContextMismatch(draft.run_id));
            }
        }

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored = load_run(&transaction, first.run_id)?;
        let sequence = match stored {
            Some(stored) => {
                validate_stored_snapshot(first.run_id, first.provider, &stored.snapshot)?;
                if stored.provider != first.provider {
                    return Err(AgentEventStoreError::ProviderMismatch {
                        expected: stored.provider,
                        received: first.provider,
                    });
                }
                if current.last_event_sequence != stored.snapshot.last_event_sequence {
                    return Err(AgentEventStoreError::StaleTracker {
                        persisted_sequence: stored.snapshot.last_event_sequence,
                        tracker_sequence: current.last_event_sequence,
                    });
                }
                let previous_json: String = transaction.query_row(
                    "SELECT event_json FROM agent_status_events
                     WHERE run_id = ?1 AND sequence = ?2",
                    params![
                        first.run_id.to_string(),
                        stored.snapshot.last_event_sequence.unwrap_or_default() as i64,
                    ],
                    |row| row.get(0),
                )?;
                let previous: AgentEvent = serde_json::from_str(&previous_json)?;
                if previous.host_id != first.host_id
                    || previous.workspace_id != first.workspace_id
                    || previous.pane_id != first.pane_id
                {
                    return Err(AgentEventStoreError::ContextMismatch(first.run_id));
                }
                stored.next_sequence
            }
            None if current.last_event_sequence.is_none() => 1,
            None => {
                return Err(AgentEventStoreError::StaleTracker {
                    persisted_sequence: None,
                    tracker_sequence: current.last_event_sequence,
                });
            }
        };
        let next_sequence = sequence
            .checked_add(drafts.len() as u64)
            .filter(|next| *next <= i64::MAX as u64)
            .ok_or(AgentEventStoreError::SequenceExhausted(first.run_id))?;

        let mut candidate = tracker.clone();
        let mut updates = Vec::with_capacity(drafts.len());
        let mut encoded_updates = Vec::with_capacity(drafts.len());
        for (offset, draft) in drafts.iter().enumerate() {
            let event = AgentEvent {
                host_id: draft.host_id,
                workspace_id: draft.workspace_id,
                run_id: draft.run_id,
                pane_id: draft.pane_id,
                provider: draft.provider,
                sequence: sequence + offset as u64,
                observed_at_ms: draft.observed_at_ms,
                source: draft.source,
                kind: draft.kind,
            };
            let snapshot = match candidate.apply_event(event.clone()) {
                EventDisposition::Applied(snapshot) => snapshot,
                EventDisposition::Ignored(reason) => {
                    return Err(AgentEventStoreError::TrackerRejected(reason));
                }
            };
            validate_stored_snapshot(draft.run_id, draft.provider, &snapshot)?;
            if snapshot.last_event_sequence != Some(event.sequence) {
                return Err(AgentEventStoreError::CorruptData(
                    "tracker snapshot did not retain the allocated sequence",
                ));
            }

            let event_json = serde_json::to_string(&event)?;
            let snapshot_json = serde_json::to_string(&snapshot)?;
            encoded_updates.push((event.sequence, event_json, snapshot_json));
            updates.push(StoredAgentUpdate { event, snapshot });
        }

        let final_snapshot = updates.last().ok_or(AgentEventStoreError::CorruptData(
            "non-empty event batch returned no update",
        ))?;
        let snapshot_json = serde_json::to_string(&final_snapshot.snapshot)?;
        transaction.execute(
            "INSERT INTO agent_status_runs
               (run_id, provider, next_sequence, latest_snapshot_json)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(run_id) DO UPDATE SET
               provider = excluded.provider,
               next_sequence = excluded.next_sequence,
               latest_snapshot_json = excluded.latest_snapshot_json",
            params![
                first.run_id.to_string(),
                first.provider.as_str(),
                next_sequence as i64,
                &snapshot_json,
            ],
        )?;
        for (sequence, event_json, snapshot_json) in encoded_updates {
            transaction.execute(
                "INSERT INTO agent_status_events
                   (run_id, sequence, event_json, snapshot_json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    first.run_id.to_string(),
                    sequence as i64,
                    event_json,
                    snapshot_json,
                ],
            )?;
        }
        transaction.commit()?;
        secure_sqlite_files(&self.path)?;

        *tracker = candidate;
        Ok(updates)
    }

    /// Appends through a tracker reconstructed from durable state. This is the
    /// path used by one-shot provider hooks, where no process-local tracker
    /// survives between events.
    pub fn append_transient(
        &mut self,
        draft: AgentEventDraft,
        needs_input_capability: super::NeedsInputCapability,
    ) -> Result<StoredAgentUpdate, AgentEventStoreError> {
        self.append_transient_batch(std::slice::from_ref(&draft), needs_input_capability)?
            .into_iter()
            .next()
            .ok_or(AgentEventStoreError::CorruptData(
                "single-event transient batch returned no update",
            ))
    }

    /// Transient counterpart to `append_batch`, reconstructing the tracker
    /// from the last committed snapshot before every concurrency retry.
    pub fn append_transient_batch(
        &mut self,
        drafts: &[AgentEventDraft],
        needs_input_capability: super::NeedsInputCapability,
    ) -> Result<Vec<StoredAgentUpdate>, AgentEventStoreError> {
        let Some(first) = drafts.first() else {
            return Ok(Vec::new());
        };
        const MAX_CONCURRENT_RELOADS: usize = 8;
        for _ in 0..MAX_CONCURRENT_RELOADS {
            let mut tracker = match self.snapshot(first.run_id)? {
                Some(snapshot) => AgentStatusTracker::from_snapshot(snapshot),
                None => {
                    AgentStatusTracker::new(first.run_id, first.provider, needs_input_capability)
                }
            };
            match self.append_batch(&mut tracker, drafts) {
                Err(AgentEventStoreError::StaleTracker { .. }) => continue,
                result => return result,
            }
        }
        Err(AgentEventStoreError::StaleTracker {
            persisted_sequence: self
                .snapshot(first.run_id)?
                .and_then(|snapshot| snapshot.last_event_sequence),
            tracker_sequence: None,
        })
    }
}
