use rusqlite::{Connection, OptionalExtension};

use crate::core::AgentRunId;

use super::{AgentEventStoreError, AgentSnapshot, Provider};

const SCHEMA_VERSION: u32 = 1;

pub(super) struct StoredRun {
    pub provider: Provider,
    pub next_sequence: u64,
    pub snapshot: AgentSnapshot,
}

pub(super) fn load_run(
    transaction: &rusqlite::Transaction<'_>,
    run_id: AgentRunId,
) -> Result<Option<StoredRun>, AgentEventStoreError> {
    let encoded = transaction
        .query_row(
            "SELECT provider, next_sequence, latest_snapshot_json
             FROM agent_status_runs WHERE run_id = ?1",
            [run_id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((provider, next_sequence, snapshot)) = encoded else {
        return Ok(None);
    };
    if next_sequence < 1 {
        return Err(AgentEventStoreError::CorruptData(
            "stored next sequence is not positive",
        ));
    }
    let snapshot = decode_snapshot(&snapshot, run_id)?;
    if snapshot
        .last_event_sequence
        .and_then(|sequence| sequence.checked_add(1))
        != Some(next_sequence as u64)
    {
        return Err(AgentEventStoreError::CorruptData(
            "stored sequence allocator does not follow the latest event",
        ));
    }
    Ok(Some(StoredRun {
        provider: provider
            .parse()
            .map_err(|_| AgentEventStoreError::CorruptData("stored provider is invalid"))?,
        next_sequence: next_sequence as u64,
        snapshot,
    }))
}

pub(super) fn decode_snapshot(
    encoded: &str,
    expected_run_id: AgentRunId,
) -> Result<AgentSnapshot, AgentEventStoreError> {
    let snapshot: AgentSnapshot = serde_json::from_str(encoded)?;
    if snapshot.run_id != expected_run_id {
        return Err(AgentEventStoreError::CorruptData(
            "stored snapshot has a different run id",
        ));
    }
    Ok(snapshot)
}

pub(super) fn validate_stored_snapshot(
    run_id: AgentRunId,
    provider: Provider,
    snapshot: &AgentSnapshot,
) -> Result<(), AgentEventStoreError> {
    if snapshot.run_id != run_id {
        return Err(AgentEventStoreError::CrossRun {
            expected: run_id,
            received: snapshot.run_id,
        });
    }
    if snapshot.provider != provider {
        return Err(AgentEventStoreError::ProviderMismatch {
            expected: provider,
            received: snapshot.provider,
        });
    }
    Ok(())
}

pub(super) fn initialize_schema(connection: &Connection) -> Result<(), AgentEventStoreError> {
    connection.execute_batch(
        "BEGIN IMMEDIATE;
         CREATE TABLE IF NOT EXISTS agent_status_schema (
           singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton = 1),
           version INTEGER NOT NULL
         );
         INSERT OR IGNORE INTO agent_status_schema (singleton, version) VALUES (1, 1);
         COMMIT;",
    )?;
    let version: u32 = connection.query_row(
        "SELECT version FROM agent_status_schema WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    if version != SCHEMA_VERSION {
        return Err(AgentEventStoreError::UnsupportedSchema {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    connection.execute_batch(
        "BEGIN IMMEDIATE;
         CREATE TABLE IF NOT EXISTS agent_status_runs (
           run_id TEXT PRIMARY KEY NOT NULL,
           provider TEXT NOT NULL,
           next_sequence INTEGER NOT NULL CHECK(next_sequence > 0),
           latest_snapshot_json TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS agent_status_events (
           run_id TEXT NOT NULL,
           sequence INTEGER NOT NULL CHECK(sequence > 0),
           event_json TEXT NOT NULL,
           snapshot_json TEXT NOT NULL,
           PRIMARY KEY (run_id, sequence),
           FOREIGN KEY (run_id) REFERENCES agent_status_runs(run_id) ON DELETE CASCADE
         );
         CREATE TABLE IF NOT EXISTS agent_integration_freshness (
           run_id TEXT PRIMARY KEY NOT NULL,
           provider TEXT NOT NULL,
           last_seen_at_ms INTEGER NOT NULL CHECK(last_seen_at_ms >= 0),
           integration_version INTEGER,
           semantic_sequence INTEGER NOT NULL DEFAULT 0 CHECK(semantic_sequence >= 0),
           delivery_gap INTEGER NOT NULL DEFAULT 0 CHECK(delivery_gap IN (0, 1))
         );
         COMMIT;",
    )?;
    // An interrupted V1 development upgrade may have created the compact
    // freshness table before delivery cursors were introduced. Such rows
    // cannot prove continuity, so migrate them fail-closed until that run is
    // restarted with a fresh launch-scoped plugin.
    if !column_exists(
        connection,
        "agent_integration_freshness",
        "semantic_sequence",
    )? {
        connection.execute(
            "ALTER TABLE agent_integration_freshness
             ADD COLUMN semantic_sequence INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if !column_exists(connection, "agent_integration_freshness", "delivery_gap")? {
        connection.execute(
            "ALTER TABLE agent_integration_freshness
             ADD COLUMN delivery_gap INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }
    Ok(())
}

fn column_exists(
    connection: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, AgentEventStoreError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    for stored in columns {
        if stored? == column {
            return Ok(true);
        }
    }
    Ok(false)
}
