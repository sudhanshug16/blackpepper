use super::{row_to_workspace, HostId, RegistryError, WorkspaceId, WorkspaceRecord};
use rusqlite::OptionalExtension;
use std::str::FromStr;

use super::WorktrunkRemovalIntent;

pub(super) fn build_intent(
    local_host_id: HostId,
    target: WorkspaceRecord,
    surviving: WorkspaceRecord,
    expected_target_path: &str,
    repository_key: String,
) -> Result<WorktrunkRemovalIntent, RegistryError> {
    if target.host_id != local_host_id || surviving.host_id != local_host_id {
        return Err(validation(
            "Worktrunk removal workspaces must belong to this host.",
        ));
    }
    if target.root_path != expected_target_path {
        return Err(validation(
            "Worktrunk target path does not match the registered workspace.",
        ));
    }
    let target_repository = target
        .repository
        .as_ref()
        .ok_or_else(|| validation("Worktrunk target has no registered Git repository identity."))?
        .repository_id();
    let surviving_repository = surviving
        .repository
        .as_ref()
        .ok_or_else(|| validation("Surviving Worktrunk workspace has no Git repository identity."))?
        .repository_id();
    if target_repository != surviving_repository {
        return Err(validation(
            "Worktrunk target and surviving workspace have different repository identities.",
        ));
    }
    Ok(WorktrunkRemovalIntent {
        workspace_id: target.id,
        surviving_workspace_id: surviving.id,
        host_id: local_host_id,
        repository_id: target_repository,
        repository_key,
        target_path: target.root_path,
        surviving_path: surviving.root_path,
    })
}

pub(super) fn validate_current_intent(
    connection: &rusqlite::Connection,
    intent: &WorktrunkRemovalIntent,
) -> Result<(), RegistryError> {
    let target = workspace_from_connection(connection, intent.workspace_id)?
        .ok_or_else(|| validation("The Worktrunk target workspace is no longer registered."))?;
    let surviving = workspace_from_connection(connection, intent.surviving_workspace_id)?
        .ok_or_else(|| validation("The surviving Worktrunk workspace is no longer registered."))?;
    let current = build_intent(
        intent.host_id,
        target,
        surviving,
        &intent.target_path,
        intent.repository_key.clone(),
    )?;
    if &current != intent {
        return Err(validation(
            "Worktrunk workspace identity changed; removal was refused.",
        ));
    }
    Ok(())
}

pub(super) fn validate_current_target(
    connection: &rusqlite::Connection,
    intent: &WorktrunkRemovalIntent,
) -> Result<(), RegistryError> {
    let target = workspace_from_connection(connection, intent.workspace_id)?
        .ok_or_else(|| validation("The Worktrunk target workspace is no longer registered."))?;
    if target.host_id != intent.host_id
        || target.root_path != intent.target_path
        || target
            .repository
            .as_ref()
            .is_none_or(|identity| identity.repository_id() != intent.repository_id)
    {
        return Err(validation(
            "Worktrunk target workspace identity changed; registry cleanup was refused.",
        ));
    }
    Ok(())
}

pub(super) fn workspace_from_connection(
    connection: &rusqlite::Connection,
    workspace_id: WorkspaceId,
) -> Result<Option<WorkspaceRecord>, RegistryError> {
    Ok(connection
        .query_row(
            &super::super::schema::workspace_select("WHERE id = ?1"),
            [workspace_id.to_string()],
            row_to_workspace,
        )
        .optional()?)
}

pub(super) fn removal_from_connection(
    connection: &rusqlite::Connection,
    workspace_id: WorkspaceId,
) -> Result<Option<WorktrunkRemovalIntent>, RegistryError> {
    Ok(connection
        .query_row(
            "SELECT workspace_id, surviving_workspace_id, host_id, repository_id,
                    repository_key, target_path, surviving_path
             FROM worktrunk_removal_intents WHERE workspace_id = ?1",
            [workspace_id.to_string()],
            row_to_removal,
        )
        .optional()?)
}

pub(super) fn row_to_removal(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorktrunkRemovalIntent> {
    Ok(WorktrunkRemovalIntent {
        workspace_id: parse_id(row.get::<_, String>(0)?, 0)?,
        surviving_workspace_id: parse_id(row.get::<_, String>(1)?, 1)?,
        host_id: parse_id(row.get::<_, String>(2)?, 2)?,
        repository_id: parse_id(row.get::<_, String>(3)?, 3)?,
        repository_key: row.get(4)?,
        target_path: row.get(5)?,
        surviving_path: row.get(6)?,
    })
}

fn parse_id<T>(value: String, index: usize) -> rusqlite::Result<T>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    value.parse().map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

pub(super) fn validation(message: impl Into<String>) -> RegistryError {
    RegistryError::Validation(message.into())
}
