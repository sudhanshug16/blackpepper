use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::core::{AgentRunId, HostId, PaneId, WorkspaceId};

use super::store_fs::{create_private_file, lock_initialization, secure_sqlite_files};
use super::store_schema::initialize_schema;
use super::{
    AgentEvent, AgentEventKind, AgentEventSource, AgentEventStoreError, AgentSnapshot, Provider,
};

/// The latest successful delivery from one launch-scoped provider adapter.
/// Only timing and version metadata are retained; heartbeat payloads never
/// enter the event log.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntegrationFreshness {
    pub provider: Provider,
    pub last_seen_at_ms: u64,
    pub integration_version: Option<u32>,
    pub semantic_sequence: u64,
    pub delivery_gap: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeliveryContinuity {
    Continuous,
    Gap,
}

/// Redacted semantic input for one provider event.
///
/// The store deliberately has no raw payload, viewport, command, or terminal
/// text parameter. Provider adapters must reduce their input to this type
/// before crossing the persistence boundary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentEventDraft {
    pub host_id: HostId,
    pub workspace_id: WorkspaceId,
    pub run_id: AgentRunId,
    pub pane_id: Option<PaneId>,
    pub provider: Provider,
    pub observed_at_ms: u64,
    pub source: AgentEventSource,
    pub kind: AgentEventKind,
}

/// An event and the snapshot produced by applying it atomically.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoredAgentUpdate {
    pub event: AgentEvent,
    pub snapshot: AgentSnapshot,
}

/// Durable provider-event log used by the host helper.
///
/// Sequence numbers are allocated inside an immediate SQLite transaction and
/// are monotonic independently for each run. The caller's tracker is updated
/// only after the event and resulting snapshot have committed together.
pub struct AgentEventStore {
    pub(super) connection: Connection,
    pub(super) path: PathBuf,
}

impl AgentEventStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AgentEventStoreError> {
        let path = path.as_ref().to_owned();
        create_private_file(&path)?;
        let _initialization_lock = lock_initialization(&path)?;
        let connection = Connection::open(&path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        let journal_mode: String =
            connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(AgentEventStoreError::UnexpectedJournalMode(journal_mode));
        }
        initialize_schema(&connection)?;
        secure_sqlite_files(&path)?;
        Ok(Self { connection, path })
    }

    pub fn journal_mode(&self) -> Result<String, AgentEventStoreError> {
        Ok(self
            .connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))?)
    }
}
