mod identity;
mod registration;
mod removals;
mod schema;
mod support;

#[cfg(test)]
mod concurrency_tests;
#[cfg(test)]
mod tests;

pub(crate) use removals::WorktrunkRemovalIntent;
pub use support::RegistryError;

use self::{
    schema::{
        host_select, initialize_schema, query_all, row_to_host, row_to_session, row_to_workspace,
        session_select, workspace_select,
    },
    support::{
        create_private_file, enable_persistent_wal, lock_registry_initialization,
        secure_sqlite_files, validate_host, validate_session, validate_workspace,
    },
};
use super::{
    HostId, HostRecord, RegistrySnapshot, SessionId, SessionRecord, WorkspaceId, WorkspaceRecord,
};
use rusqlite::{Connection, OptionalExtension};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub struct HostRegistry {
    connection: Connection,
    path: PathBuf,
}

impl HostRegistry {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RegistryError> {
        Self::open_interruptible(path, || false)
    }

    /// Open a second WAL connection while allowing a background owner to
    /// abandon a contended schema-initialization lock during shutdown.
    pub(crate) fn open_interruptible(
        path: impl AsRef<Path>,
        mut interrupted: impl FnMut() -> bool,
    ) -> Result<Self, RegistryError> {
        let path = path.as_ref().to_owned();
        create_private_file(&path)?;
        let _initialization_lock = lock_registry_initialization(&path, &mut interrupted)?;
        if interrupted() {
            return Err(RegistryError::Interrupted(
                "registry initialization was cancelled".to_owned(),
            ));
        }

        let connection = Connection::open(&path)?;
        // Initialization belongs to a cancellable restore worker. A short
        // SQLite busy deadline keeps shutdown bounded; ordinary operations
        // regain the normal five-second contention window after setup.
        connection.busy_timeout(Duration::from_millis(250))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        let journal_mode: String =
            connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(RegistryError::UnexpectedValue(format!(
                "SQLite refused WAL mode and selected {journal_mode}"
            )));
        }
        enable_persistent_wal(&connection)?;
        initialize_schema(&connection)?;
        connection.busy_timeout(Duration::from_secs(5))?;

        let registry = Self { connection, path };
        secure_sqlite_files(&registry.path)?;
        Ok(registry)
    }

    /// Open the already-initialized client registry for a transient worker.
    /// The interactive runtime owns schema initialization, so this path never
    /// takes the initialization flock or asks SQLite to change journal mode.
    pub(crate) fn open_existing_interruptible(
        path: impl AsRef<Path>,
        mut interrupted: impl FnMut() -> bool,
    ) -> Result<Self, RegistryError> {
        let path = path.as_ref().to_owned();
        create_private_file(&path)?;
        if interrupted() {
            return Err(RegistryError::Interrupted(
                "registry connection was cancelled".to_owned(),
            ));
        }
        let connection = Connection::open(&path)?;
        connection.busy_timeout(Duration::from_millis(250))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        let journal_mode: String =
            connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(RegistryError::UnexpectedValue(format!(
                "existing registry is not in WAL mode (found {journal_mode})"
            )));
        }
        enable_persistent_wal(&connection)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let registry = Self { connection, path };
        secure_sqlite_files(&registry.path)?;
        Ok(registry)
    }

    pub fn upsert_host(&self, host: &HostRecord) -> Result<(), RegistryError> {
        validate_host(host)?;
        let transport = serde_json::to_string(&host.transport)?;
        self.connection.execute(
            "INSERT INTO hosts (id, display_name, transport_json, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
               display_name = excluded.display_name,
               transport_json = excluded.transport_json,
               created_at_ms = excluded.created_at_ms,
               updated_at_ms = excluded.updated_at_ms",
            (
                host.id.to_string(),
                &host.display_name,
                transport,
                host.created_at_ms,
                host.updated_at_ms,
            ),
        )?;
        secure_sqlite_files(&self.path)?;
        Ok(())
    }

    pub fn upsert_workspace(&self, workspace: &WorkspaceRecord) -> Result<(), RegistryError> {
        validate_workspace(workspace)?;
        let repository = workspace
            .repository
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let grouping = serde_json::to_string(&workspace.grouping)?;
        let setup = serde_json::to_string(&workspace.setup)?;
        let repository_id = workspace.repository_id().map(|id| id.to_string());
        self.connection.execute(
            "INSERT INTO workspaces
               (id, host_id, root_path, display_name, repository_json, grouping_json, setup_json,
                repository_id, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
               host_id = excluded.host_id,
               root_path = excluded.root_path,
               display_name = excluded.display_name,
               repository_json = excluded.repository_json,
               grouping_json = excluded.grouping_json,
               setup_json = excluded.setup_json,
               repository_id = excluded.repository_id,
               created_at_ms = excluded.created_at_ms,
               updated_at_ms = excluded.updated_at_ms",
            (
                workspace.id.to_string(),
                workspace.host_id.to_string(),
                &workspace.root_path,
                &workspace.display_name,
                repository,
                grouping,
                setup,
                repository_id,
                workspace.created_at_ms,
                workspace.updated_at_ms,
            ),
        )?;
        secure_sqlite_files(&self.path)?;
        Ok(())
    }

    pub fn upsert_session(&self, session: &SessionRecord) -> Result<(), RegistryError> {
        validate_session(session)?;
        self.connection.execute(
            "INSERT INTO sessions
               (id, workspace_id, backend_json, backend_version, backend_session_id, state_json,
                created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
               workspace_id = excluded.workspace_id,
               backend_json = excluded.backend_json,
               backend_version = excluded.backend_version,
               backend_session_id = excluded.backend_session_id,
               state_json = excluded.state_json,
               created_at_ms = excluded.created_at_ms,
               updated_at_ms = excluded.updated_at_ms",
            (
                session.id.to_string(),
                session.workspace_id.to_string(),
                serde_json::to_string(&session.backend)?,
                &session.backend_version,
                &session.backend_session_id,
                serde_json::to_string(&session.state)?,
                session.created_at_ms,
                session.updated_at_ms,
            ),
        )?;
        secure_sqlite_files(&self.path)?;
        Ok(())
    }

    pub fn host(&self, id: HostId) -> Result<Option<HostRecord>, RegistryError> {
        Ok(self
            .connection
            .query_row(&host_select("WHERE id = ?1"), [id.to_string()], row_to_host)
            .optional()?)
    }

    pub fn workspace(&self, id: WorkspaceId) -> Result<Option<WorkspaceRecord>, RegistryError> {
        Ok(self
            .connection
            .query_row(
                &workspace_select("WHERE id = ?1"),
                [id.to_string()],
                row_to_workspace,
            )
            .optional()?)
    }

    pub fn session(&self, id: SessionId) -> Result<Option<SessionRecord>, RegistryError> {
        Ok(self
            .connection
            .query_row(
                &session_select("WHERE id = ?1"),
                [id.to_string()],
                row_to_session,
            )
            .optional()?)
    }

    pub fn sessions_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<SessionRecord>, RegistryError> {
        let mut statement = self.connection.prepare(&session_select(
            "WHERE workspace_id = ?1 ORDER BY created_at_ms, id",
        ))?;
        let rows = statement.query_map([workspace_id.to_string()], row_to_session)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn snapshot(&self) -> Result<RegistrySnapshot, RegistryError> {
        let snapshot = RegistrySnapshot {
            hosts: query_all(
                &self.connection,
                &host_select("ORDER BY display_name, id"),
                row_to_host,
            )?,
            workspaces: query_all(
                &self.connection,
                &workspace_select("ORDER BY host_id, root_path, id"),
                row_to_workspace,
            )?,
            sessions: query_all(
                &self.connection,
                &session_select("ORDER BY workspace_id, created_at_ms, id"),
                row_to_session,
            )?,
            pending_worktree_removals: self.pending_worktree_removal_ids()?,
        };
        secure_sqlite_files(&self.path)?;
        Ok(snapshot)
    }

    pub fn remove_host(&self, id: HostId) -> Result<bool, RegistryError> {
        self.remove("DELETE FROM hosts WHERE id = ?1", id.to_string())
    }

    pub fn remove_workspace(&self, id: WorkspaceId) -> Result<bool, RegistryError> {
        self.remove("DELETE FROM workspaces WHERE id = ?1", id.to_string())
    }

    pub fn remove_session(&self, id: SessionId) -> Result<bool, RegistryError> {
        self.remove("DELETE FROM sessions WHERE id = ?1", id.to_string())
    }

    pub fn journal_mode(&self) -> Result<String, RegistryError> {
        Ok(self
            .connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))?)
    }

    fn remove(&self, sql: &str, id: String) -> Result<bool, RegistryError> {
        let removed = self.connection.execute(sql, [id])? > 0;
        secure_sqlite_files(&self.path)?;
        Ok(removed)
    }
}
