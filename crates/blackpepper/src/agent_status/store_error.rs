use std::{error::Error, fmt};

use crate::core::AgentRunId;

use super::{IgnoredUpdate, Provider};

#[derive(Debug)]
pub enum AgentEventStoreError {
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    Serialization(serde_json::Error),
    CrossRun {
        expected: AgentRunId,
        received: AgentRunId,
    },
    ProviderMismatch {
        expected: Provider,
        received: Provider,
    },
    ContextMismatch(AgentRunId),
    StaleTracker {
        persisted_sequence: Option<u64>,
        tracker_sequence: Option<u64>,
    },
    TrackerRejected(IgnoredUpdate),
    SequenceExhausted(AgentRunId),
    UnexpectedJournalMode(String),
    UnsupportedSchema {
        found: u32,
        supported: u32,
    },
    CorruptData(&'static str),
}

impl fmt::Display for AgentEventStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "agent event storage error: {error}"),
            Self::Sqlite(error) => write!(formatter, "agent event database error: {error}"),
            Self::Serialization(error) => write!(formatter, "agent event encoding error: {error}"),
            Self::CrossRun { expected, received } => {
                write!(formatter, "stale agent run {received}; active run is {expected}")
            }
            Self::ProviderMismatch { expected, received } => write!(
                formatter,
                "agent provider {received} does not match active provider {expected}"
            ),
            Self::ContextMismatch(run_id) => {
                write!(formatter, "agent event context changed within run {run_id}")
            }
            Self::StaleTracker {
                persisted_sequence,
                tracker_sequence,
            } => write!(
                formatter,
                "stale agent tracker at {tracker_sequence:?}; persisted sequence is {persisted_sequence:?}"
            ),
            Self::TrackerRejected(reason) => {
                write!(formatter, "agent tracker rejected persisted event: {reason:?}")
            }
            Self::SequenceExhausted(run_id) => {
                write!(formatter, "agent event sequence exhausted for run {run_id}")
            }
            Self::UnexpectedJournalMode(mode) => {
                write!(formatter, "SQLite refused WAL mode and selected {mode}")
            }
            Self::UnsupportedSchema { found, supported } => write!(
                formatter,
                "unsupported agent event schema {found}; this build supports {supported}"
            ),
            Self::CorruptData(message) => write!(formatter, "corrupt agent event data: {message}"),
        }
    }
}

impl Error for AgentEventStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::Serialization(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for AgentEventStoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for AgentEventStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<serde_json::Error> for AgentEventStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}
