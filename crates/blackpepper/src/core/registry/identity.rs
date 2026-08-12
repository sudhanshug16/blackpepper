use super::{support::secure_sqlite_files, HostRegistry, RegistryError};
use crate::core::{HostId, HostRecord, HostTransport};
use rusqlite::{OptionalExtension, TransactionBehavior};

impl HostRegistry {
    /// Returns the stable identity of this installation, creating it atomically when absent.
    pub fn ensure_local_host(&mut self, display_name: &str) -> Result<HostId, RegistryError> {
        if display_name.trim().is_empty() {
            return Err(RegistryError::Validation(
                "local host display name cannot be empty".to_owned(),
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(value) = transaction
            .query_row(
                "SELECT local_host_id FROM registry_metadata WHERE singleton = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            let id = parse_host_id(&value)?;
            transaction.commit()?;
            return Ok(id);
        }

        let local_transport = serde_json::to_string(&HostTransport::Local)?;
        let existing = transaction
            .query_row(
                "SELECT id FROM hosts WHERE transport_json = ?1 ORDER BY created_at_ms, id LIMIT 1",
                [&local_transport],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let id = match existing {
            Some(value) => parse_host_id(&value)?,
            None => {
                let host = HostRecord::new(display_name.trim(), HostTransport::Local);
                transaction.execute(
                    "INSERT INTO hosts
                       (id, display_name, transport_json, created_at_ms, updated_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    (
                        host.id.to_string(),
                        &host.display_name,
                        &local_transport,
                        host.created_at_ms,
                        host.updated_at_ms,
                    ),
                )?;
                host.id
            }
        };
        transaction.execute(
            "INSERT INTO registry_metadata (singleton, local_host_id) VALUES (1, ?1)",
            [id.to_string()],
        )?;
        transaction.commit()?;
        secure_sqlite_files(&self.path)?;
        Ok(id)
    }

    pub fn local_host_id(&self) -> Result<HostId, RegistryError> {
        let value = self
            .connection
            .query_row(
                "SELECT local_host_id FROM registry_metadata WHERE singleton = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| {
                RegistryError::Validation(
                    "local host identity has not been initialized; call ensure_local_host first"
                        .to_owned(),
                )
            })?;
        parse_host_id(&value)
    }
}

fn parse_host_id(value: &str) -> Result<HostId, RegistryError> {
    value.parse().map_err(|error| {
        RegistryError::UnexpectedValue(format!("registry contains invalid local host ID: {error}"))
    })
}
