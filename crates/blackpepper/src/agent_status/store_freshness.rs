use rusqlite::{params, OptionalExtension, TransactionBehavior};

use crate::core::AgentRunId;

use super::store::{DeliveryContinuity, IntegrationFreshness};
use super::store_fs::secure_sqlite_files;
use super::{AgentEventStore, AgentEventStoreError, Provider};

impl AgentEventStore {
    /// Atomically advances the compact freshness cursor for one integration.
    /// Repeated heartbeats update this single row instead of growing the
    /// semantic event log.
    pub(crate) fn touch_integration(
        &mut self,
        run_id: AgentRunId,
        provider: Provider,
        observed_at_ms: u64,
        integration_version: Option<u32>,
        reported_semantic_sequence: u64,
    ) -> Result<DeliveryContinuity, AgentEventStoreError> {
        let observed_at_ms = observed_at_ms.min(i64::MAX as u64) as i64;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT provider, semantic_sequence, delivery_gap
                 FROM agent_integration_freshness WHERE run_id = ?1",
                [run_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                },
            )
            .optional()?;
        let (semantic_sequence, delivery_gap) = if let Some((existing, sequence, gap)) = existing {
            let existing = existing.parse::<Provider>().map_err(|_| {
                AgentEventStoreError::CorruptData("stored integration provider is invalid")
            })?;
            if existing != provider {
                return Err(AgentEventStoreError::ProviderMismatch {
                    expected: existing,
                    received: provider,
                });
            }
            if sequence < 0 {
                return Err(AgentEventStoreError::CorruptData(
                    "stored integration cursor is invalid",
                ));
            }
            (sequence as u64, gap)
        } else {
            (0, false)
        };
        let continuous = !delivery_gap && reported_semantic_sequence == semantic_sequence;
        transaction.execute(
            "INSERT INTO agent_integration_freshness
               (run_id, provider, last_seen_at_ms, integration_version,
                semantic_sequence, delivery_gap)
             VALUES (?1, ?2, ?3, ?4, 0, ?5)
             ON CONFLICT(run_id) DO UPDATE SET
               last_seen_at_ms = CASE WHEN ?5 = 0
                 THEN MAX(last_seen_at_ms, excluded.last_seen_at_ms)
                 ELSE last_seen_at_ms END,
               integration_version = CASE WHEN ?5 = 0
                 THEN COALESCE(excluded.integration_version, integration_version)
                 ELSE integration_version END,
               delivery_gap = MAX(delivery_gap, ?5)",
            params![
                run_id.to_string(),
                provider.as_str(),
                observed_at_ms,
                integration_version.map(i64::from),
                i64::from(!continuous),
            ],
        )?;
        transaction.commit()?;
        secure_sqlite_files(&self.path)?;
        Ok(if continuous {
            DeliveryContinuity::Continuous
        } else {
            DeliveryContinuity::Gap
        })
    }

    /// Commit the next exact semantic cursor only after its state batch has
    /// committed. Missing, duplicate, or out-of-order cursors permanently
    /// fail this launch-scoped integration closed.
    pub(crate) fn advance_integration(
        &mut self,
        run_id: AgentRunId,
        provider: Provider,
        observed_at_ms: u64,
        integration_version: Option<u32>,
        semantic_sequence: u64,
    ) -> Result<DeliveryContinuity, AgentEventStoreError> {
        let observed_at_ms = observed_at_ms.min(i64::MAX as u64) as i64;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT provider, semantic_sequence, delivery_gap
                 FROM agent_integration_freshness WHERE run_id = ?1",
                [run_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((existing_provider, committed_sequence, delivery_gap)) = existing else {
            return Ok(DeliveryContinuity::Gap);
        };
        let existing_provider = existing_provider.parse::<Provider>().map_err(|_| {
            AgentEventStoreError::CorruptData("stored integration provider is invalid")
        })?;
        if existing_provider != provider {
            return Err(AgentEventStoreError::ProviderMismatch {
                expected: existing_provider,
                received: provider,
            });
        }
        if committed_sequence < 0 {
            return Err(AgentEventStoreError::CorruptData(
                "stored integration cursor is invalid",
            ));
        }
        let continuous = semantic_sequence <= i64::MAX as u64
            && !delivery_gap
            && semantic_sequence == (committed_sequence as u64).saturating_add(1);
        if continuous {
            transaction.execute(
                "UPDATE agent_integration_freshness SET
                   last_seen_at_ms = MAX(last_seen_at_ms, ?2),
                   integration_version = COALESCE(?3, integration_version),
                   semantic_sequence = ?4
                 WHERE run_id = ?1 AND delivery_gap = 0",
                params![
                    run_id.to_string(),
                    observed_at_ms,
                    integration_version.map(i64::from),
                    semantic_sequence.min(i64::MAX as u64) as i64,
                ],
            )?;
        } else {
            transaction.execute(
                "UPDATE agent_integration_freshness
                 SET delivery_gap = 1 WHERE run_id = ?1",
                [run_id.to_string()],
            )?;
        }
        transaction.commit()?;
        secure_sqlite_files(&self.path)?;
        Ok(if continuous {
            DeliveryContinuity::Continuous
        } else {
            DeliveryContinuity::Gap
        })
    }

    pub(crate) fn mark_integration_gap(
        &mut self,
        run_id: AgentRunId,
        provider: Provider,
        integration_version: Option<u32>,
    ) -> Result<(), AgentEventStoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT provider FROM agent_integration_freshness WHERE run_id = ?1",
                [run_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let existing = existing.parse::<Provider>().map_err(|_| {
                AgentEventStoreError::CorruptData("stored integration provider is invalid")
            })?;
            if existing != provider {
                return Err(AgentEventStoreError::ProviderMismatch {
                    expected: existing,
                    received: provider,
                });
            }
        }
        transaction.execute(
            "INSERT INTO agent_integration_freshness
               (run_id, provider, last_seen_at_ms, integration_version,
                semantic_sequence, delivery_gap)
             VALUES (?1, ?2, 0, ?3, 0, 1)
             ON CONFLICT(run_id) DO UPDATE SET delivery_gap = 1",
            params![
                run_id.to_string(),
                provider.as_str(),
                integration_version.map(i64::from),
            ],
        )?;
        transaction.commit()?;
        secure_sqlite_files(&self.path)?;
        Ok(())
    }

    pub(crate) fn integration_freshness(
        &self,
        run_id: AgentRunId,
    ) -> Result<Option<IntegrationFreshness>, AgentEventStoreError> {
        let encoded = self
            .connection
            .query_row(
                "SELECT provider, last_seen_at_ms, integration_version,
                        semantic_sequence, delivery_gap
                 FROM agent_integration_freshness WHERE run_id = ?1",
                [run_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, bool>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((provider, last_seen_at_ms, integration_version, semantic_sequence, delivery_gap)) =
            encoded
        else {
            return Ok(None);
        };
        if last_seen_at_ms < 0
            || semantic_sequence < 0
            || integration_version.is_some_and(|version| !(0..=u32::MAX as i64).contains(&version))
        {
            return Err(AgentEventStoreError::CorruptData(
                "stored integration freshness is invalid",
            ));
        }
        Ok(Some(IntegrationFreshness {
            provider: provider.parse().map_err(|_| {
                AgentEventStoreError::CorruptData("stored integration provider is invalid")
            })?,
            last_seen_at_ms: last_seen_at_ms as u64,
            integration_version: integration_version.map(|version| version as u32),
            semantic_sequence: semantic_sequence as u64,
            delivery_gap,
        }))
    }
}
