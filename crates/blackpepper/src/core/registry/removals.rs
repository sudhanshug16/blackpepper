use super::{schema::row_to_workspace, support::secure_sqlite_files, HostRegistry, RegistryError};
use crate::core::{HostId, RepositoryId, WorkspaceId, WorkspaceRecord};
use rusqlite::{Transaction, TransactionBehavior};
use std::{path::Path, str::FromStr, time::SystemTime};

mod validation;
use validation::{
    build_intent, removal_from_connection, row_to_removal, validate_current_intent,
    validate_current_target, validation, workspace_from_connection,
};

/// Durable evidence that a Worktrunk removal was dispatched but may not yet
/// have been reflected in the workspace registry. This record deliberately
/// has no foreign keys: it must survive deletion of the workspace it repairs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorktrunkRemovalIntent {
    pub(crate) workspace_id: WorkspaceId,
    pub(crate) surviving_workspace_id: WorkspaceId,
    pub(crate) host_id: HostId,
    pub(crate) repository_id: RepositoryId,
    pub(crate) repository_key: String,
    pub(crate) target_path: String,
    pub(crate) surviving_path: String,
}

impl HostRegistry {
    /// Validates stable IDs against the current records before Worktrunk argv
    /// is built. UI grouping overrides are intentionally ignored here: Git
    /// repository identity, not presentation, controls destructive actions.
    pub(crate) fn plan_worktrunk_removal(
        &self,
        workspace_id: WorkspaceId,
        surviving_workspace_id: WorkspaceId,
        expected_target_path: &str,
        repository_key: String,
    ) -> Result<WorktrunkRemovalIntent, RegistryError> {
        if workspace_id == surviving_workspace_id {
            return Err(validation(
                "Worktrunk removal requires a different surviving workspace.",
            ));
        }
        if !Path::new(&repository_key).is_absolute() {
            return Err(validation(
                "Worktrunk repository identity must be an absolute path.",
            ));
        }
        if self.worktrunk_removal(workspace_id)?.is_some() {
            return Err(validation(
                "A previous Worktrunk removal has an unknown result; run :worktree list before trying again.",
            ));
        }
        let target = self
            .workspace(workspace_id)?
            .ok_or_else(|| validation("The Worktrunk target workspace is not registered."))?;
        let surviving = self
            .workspace(surviving_workspace_id)?
            .ok_or_else(|| validation("The surviving Worktrunk workspace is not registered."))?;
        build_intent(
            self.local_host_id()?,
            target,
            surviving,
            expected_target_path,
            repository_key,
        )
    }

    /// Persists the removal marker under an IMMEDIATE SQLite transaction and
    /// revalidates every ID/path/repository field. A conflicting marker fails
    /// closed so an ambiguous mutation can never be retried implicitly.
    pub(crate) fn journal_worktrunk_removal(
        &self,
        intent: &WorktrunkRemovalIntent,
    ) -> Result<(), RegistryError> {
        let transaction =
            Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        if removal_from_connection(&transaction, intent.workspace_id)?.is_some() {
            return Err(validation(
                "A previous Worktrunk removal has an unknown result; run :worktree list before trying again.",
            ));
        }
        validate_current_intent(&transaction, intent)?;
        transaction.execute(
            "INSERT INTO worktrunk_removal_intents
               (workspace_id, surviving_workspace_id, host_id, repository_id, repository_key,
                target_path, surviving_path, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                intent.workspace_id.to_string(),
                intent.surviving_workspace_id.to_string(),
                intent.host_id.to_string(),
                intent.repository_id.to_string(),
                &intent.repository_key,
                &intent.target_path,
                &intent.surviving_path,
                now_millis(),
            ),
        )?;
        transaction.commit()?;
        secure_sqlite_files(&self.path)
    }

    pub(crate) fn worktrunk_removal(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorktrunkRemovalIntent>, RegistryError> {
        removal_from_connection(&self.connection, workspace_id)
    }

    pub(crate) fn worktrunk_removals_for_repository(
        &self,
        repository_key: &str,
    ) -> Result<Vec<WorktrunkRemovalIntent>, RegistryError> {
        let mut statement = self.connection.prepare(
            "SELECT workspace_id, surviving_workspace_id, host_id, repository_id,
                    repository_key, target_path, surviving_path
             FROM worktrunk_removal_intents
             WHERE repository_key = ?1
             ORDER BY created_at_ms, workspace_id",
        )?;
        let rows = statement.query_map([repository_key], row_to_removal)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn pending_worktree_removal_ids(&self) -> Result<Vec<WorkspaceId>, RegistryError> {
        let mut statement = self.connection.prepare(
            "SELECT workspace_id FROM worktrunk_removal_intents
             ORDER BY created_at_ms, workspace_id",
        )?;
        let rows = statement.query_map([], |row| {
            let value: String = row.get(0)?;
            WorkspaceId::from_str(&value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Atomically deletes the workspace and its recovery marker. If another
    /// client already deleted the workspace, clearing the exact marker remains
    /// idempotent; a changed workspace record is never deleted.
    pub(crate) fn finish_worktrunk_removal(
        &self,
        intent: &WorktrunkRemovalIntent,
    ) -> Result<bool, RegistryError> {
        let transaction =
            Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let stored = removal_from_connection(&transaction, intent.workspace_id)?
            .ok_or_else(|| validation("Worktrunk removal recovery marker is missing."))?;
        if &stored != intent {
            return Err(validation(
                "Worktrunk removal recovery marker changed; registry cleanup was refused.",
            ));
        }
        let removed = match workspace_from_connection(&transaction, intent.workspace_id)? {
            Some(_) => {
                validate_current_target(&transaction, intent)?;
                transaction.execute(
                    "DELETE FROM workspaces WHERE id = ?1",
                    [intent.workspace_id.to_string()],
                )? > 0
            }
            None => false,
        };
        let cleared = transaction.execute(
            "DELETE FROM worktrunk_removal_intents WHERE workspace_id = ?1",
            [intent.workspace_id.to_string()],
        )?;
        if cleared != 1 {
            return Err(validation(
                "Worktrunk removal recovery marker could not be cleared.",
            ));
        }
        transaction.commit()?;
        secure_sqlite_files(&self.path)?;
        Ok(removed)
    }

    /// Clears a marker only after a fresh schema-2 list proves the target is
    /// still present, or when the Worktrunk process could not be started.
    pub(crate) fn cancel_worktrunk_removal(
        &self,
        intent: &WorktrunkRemovalIntent,
    ) -> Result<(), RegistryError> {
        let transaction =
            Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let stored = removal_from_connection(&transaction, intent.workspace_id)?
            .ok_or_else(|| validation("Worktrunk removal recovery marker is missing."))?;
        if &stored != intent {
            return Err(validation(
                "Worktrunk removal recovery marker changed; cancellation was refused.",
            ));
        }
        transaction.execute(
            "DELETE FROM worktrunk_removal_intents WHERE workspace_id = ?1",
            [intent.workspace_id.to_string()],
        )?;
        transaction.commit()?;
        secure_sqlite_files(&self.path)
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}
