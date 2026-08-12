use super::RegistryError;
use crate::core::{HostRecord, SessionRecord, WorkspaceRecord};
use rusqlite::{types::Type, Connection, Row};
use std::{error::Error, str::FromStr};

const SCHEMA_VERSION: u32 = 5;

pub(super) fn initialize_schema(connection: &Connection) -> Result<(), RegistryError> {
    connection.execute_batch("BEGIN IMMEDIATE")?;
    let result = initialize_schema_locked(connection);
    match result {
        Ok(()) => {
            connection.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

fn initialize_schema_locked(connection: &Connection) -> Result<(), RegistryError> {
    let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(RegistryError::UnsupportedSchema {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    if version == 0 {
        connection.execute_batch(
            "CREATE TABLE hosts (
               id TEXT PRIMARY KEY NOT NULL,
               display_name TEXT NOT NULL,
               transport_json TEXT NOT NULL,
               created_at_ms INTEGER NOT NULL,
               updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE registry_metadata (
               singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
               local_host_id TEXT NOT NULL REFERENCES hosts(id)
             );
             CREATE TABLE workspaces (
               id TEXT PRIMARY KEY NOT NULL,
               host_id TEXT NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
               root_path TEXT NOT NULL,
               display_name TEXT,
               repository_json TEXT,
               grouping_json TEXT NOT NULL,
               setup_json TEXT NOT NULL,
               repository_id TEXT,
               created_at_ms INTEGER NOT NULL,
               updated_at_ms INTEGER NOT NULL,
               UNIQUE(host_id, root_path)
             );
             CREATE INDEX workspaces_repository_id ON workspaces(repository_id);
             CREATE TABLE sessions (
               id TEXT PRIMARY KEY NOT NULL,
               workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
               backend_json TEXT NOT NULL,
               backend_version TEXT NOT NULL,
               backend_session_id TEXT NOT NULL,
               state_json TEXT NOT NULL,
               created_at_ms INTEGER NOT NULL,
               updated_at_ms INTEGER NOT NULL,
               UNIQUE(workspace_id, backend_json, backend_session_id)
             );
             CREATE TABLE worktrunk_removal_intents (
               workspace_id TEXT PRIMARY KEY NOT NULL,
               surviving_workspace_id TEXT NOT NULL,
               host_id TEXT NOT NULL,
               repository_id TEXT NOT NULL,
               repository_key TEXT NOT NULL,
               target_path TEXT NOT NULL,
               surviving_path TEXT NOT NULL,
               created_at_ms INTEGER NOT NULL
             );
             CREATE INDEX worktrunk_removal_repository_key
               ON worktrunk_removal_intents(repository_key);
             PRAGMA user_version = 5;",
        )?;
    } else if version == 1 {
        connection.execute_batch(
            "CREATE TABLE registry_metadata (
               singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
               local_host_id TEXT NOT NULL REFERENCES hosts(id)
             );
             ALTER TABLE sessions ADD COLUMN backend_version TEXT NOT NULL DEFAULT 'unknown';
             ALTER TABLE workspaces ADD COLUMN setup_json TEXT NOT NULL DEFAULT '{\"status\":\"ready\"}';",
        )?;
    } else if version == 2 {
        connection.execute_batch(
            "ALTER TABLE sessions ADD COLUMN backend_version TEXT NOT NULL DEFAULT 'unknown';
             ALTER TABLE workspaces ADD COLUMN setup_json TEXT NOT NULL DEFAULT '{\"status\":\"ready\"}';",
        )?;
    } else if version == 3 {
        connection.execute_batch(
            "ALTER TABLE workspaces ADD COLUMN setup_json TEXT NOT NULL DEFAULT '{\"status\":\"ready\"}';",
        )?;
    }
    if (1..=4).contains(&version) {
        connection.execute_batch(
            "CREATE TABLE worktrunk_removal_intents (
               workspace_id TEXT PRIMARY KEY NOT NULL,
               surviving_workspace_id TEXT NOT NULL,
               host_id TEXT NOT NULL,
               repository_id TEXT NOT NULL,
               repository_key TEXT NOT NULL,
               target_path TEXT NOT NULL,
               surviving_path TEXT NOT NULL,
               created_at_ms INTEGER NOT NULL
             );
             CREATE INDEX worktrunk_removal_repository_key
               ON worktrunk_removal_intents(repository_key);
             PRAGMA user_version = 5;",
        )?;
    }
    Ok(())
}

pub(super) fn host_select(suffix: &str) -> String {
    format!(
        "SELECT id, display_name, transport_json, created_at_ms, updated_at_ms FROM hosts {suffix}"
    )
}

pub(super) fn workspace_select(suffix: &str) -> String {
    format!(
        "SELECT id, host_id, root_path, display_name, repository_json, grouping_json, setup_json,
         created_at_ms, updated_at_ms FROM workspaces {suffix}"
    )
}

pub(super) fn session_select(suffix: &str) -> String {
    format!(
        "SELECT id, workspace_id, backend_json, backend_version, backend_session_id, state_json,
         created_at_ms, updated_at_ms FROM sessions {suffix}"
    )
}

pub(super) fn query_all<T>(
    connection: &Connection,
    sql: &str,
    map: fn(&Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>, RegistryError> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map([], map)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub(super) fn row_to_host(row: &Row<'_>) -> rusqlite::Result<HostRecord> {
    Ok(HostRecord {
        id: parse_id(row, 0)?,
        display_name: row.get(1)?,
        transport: parse_json(row, 2)?,
        created_at_ms: row.get(3)?,
        updated_at_ms: row.get(4)?,
    })
}

pub(super) fn row_to_workspace(row: &Row<'_>) -> rusqlite::Result<WorkspaceRecord> {
    let repository_json: Option<String> = row.get(4)?;
    Ok(WorkspaceRecord {
        id: parse_id(row, 0)?,
        host_id: parse_id(row, 1)?,
        root_path: row.get(2)?,
        display_name: row.get(3)?,
        repository: repository_json
            .map(|value| serde_json::from_str(&value).map_err(|error| conversion_error(4, error)))
            .transpose()?,
        grouping: parse_json(row, 5)?,
        setup: parse_json(row, 6)?,
        created_at_ms: row.get(7)?,
        updated_at_ms: row.get(8)?,
    })
}

pub(super) fn row_to_session(row: &Row<'_>) -> rusqlite::Result<SessionRecord> {
    Ok(SessionRecord {
        id: parse_id(row, 0)?,
        workspace_id: parse_id(row, 1)?,
        backend: parse_json(row, 2)?,
        backend_version: row.get(3)?,
        backend_session_id: row.get(4)?,
        state: parse_json(row, 5)?,
        created_at_ms: row.get(6)?,
        updated_at_ms: row.get(7)?,
    })
}

fn parse_id<T>(row: &Row<'_>, index: usize) -> rusqlite::Result<T>
where
    T: FromStr,
    T::Err: Error + Send + Sync + 'static,
{
    let value: String = row.get(index)?;
    value
        .parse()
        .map_err(|error| conversion_error(index, error))
}

fn parse_json<T: serde::de::DeserializeOwned>(row: &Row<'_>, index: usize) -> rusqlite::Result<T> {
    let value: String = row.get(index)?;
    serde_json::from_str(&value).map_err(|error| conversion_error(index, error))
}

fn conversion_error(index: usize, error: impl Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_version_two_sessions_with_an_explicit_legacy_marker() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sessions (id TEXT PRIMARY KEY);
                 CREATE TABLE workspaces (id TEXT PRIMARY KEY);
                 PRAGMA user_version = 2;",
            )
            .unwrap();
        initialize_schema(&connection).unwrap();

        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let default_value: String = connection
            .query_row(
                "SELECT dflt_value FROM pragma_table_info('sessions') WHERE name = 'backend_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let setup_default: String = connection
            .query_row(
                "SELECT dflt_value FROM pragma_table_info('workspaces') WHERE name = 'setup_json'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 5);
        assert_eq!(default_value, "'unknown'");
        assert_eq!(setup_default, "'{\"status\":\"ready\"}'");
    }

    #[test]
    fn migrates_version_four_with_a_durable_worktrunk_removal_journal() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("PRAGMA user_version = 4;")
            .unwrap();
        initialize_schema(&connection).unwrap();

        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let table: String = connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'worktrunk_removal_intents'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 5);
        assert_eq!(table, "worktrunk_removal_intents");
    }
}
