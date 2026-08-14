use super::{
    schema::row_to_workspace,
    support::{secure_sqlite_files, validate_workspace},
    HostRegistry, RegistryError,
};
use crate::core::WorkspaceRecord;

impl HostRegistry {
    /// Inserts a newly discovered workspace or returns the row another client
    /// registered for the same host and canonical root. The conflict update is
    /// deliberately a no-op: registration must not overwrite settings owned by
    /// an already-registered workspace.
    pub fn insert_workspace_or_existing(
        &self,
        workspace: &WorkspaceRecord,
    ) -> Result<WorkspaceRecord, RegistryError> {
        validate_workspace(workspace)?;
        let repository = workspace
            .repository
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let grouping = serde_json::to_string(&workspace.grouping)?;
        let setup = serde_json::to_string(&workspace.setup)?;
        let repository_id = workspace.repository_id().map(|id| id.to_string());
        let registered = self.connection.query_row(
            "INSERT INTO workspaces
               (id, host_id, root_path, display_name, repository_json, grouping_json, setup_json,
                repository_id, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(host_id, root_path) DO UPDATE SET
               root_path = excluded.root_path
             RETURNING id, host_id, root_path, display_name, repository_json, grouping_json,
               setup_json, created_at_ms, updated_at_ms",
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
            row_to_workspace,
        )?;
        // A successful SQLite write is not enough: callers must still see a
        // permissions failure instead of silently accepting exposed state.
        secure_sqlite_files(&self.path)?;
        Ok(registered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn duplicate_registration_preserves_the_first_records_settings() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("registry.sqlite3");
        let mut registry = HostRegistry::open(&path).unwrap();
        let host_id = registry.ensure_local_host("local").unwrap();
        let mut first = WorkspaceRecord::new(host_id, "/srv/shared");
        first.display_name = Some("Production".to_owned());
        let mut duplicate = WorkspaceRecord::new(host_id, "/srv/shared");
        duplicate.display_name = Some("Development".to_owned());

        assert_eq!(
            registry.insert_workspace_or_existing(&first).unwrap(),
            first
        );
        assert_eq!(
            registry.insert_workspace_or_existing(&duplicate).unwrap(),
            first
        );
        assert_eq!(registry.snapshot().unwrap().workspaces, vec![first]);
    }

    #[test]
    fn concurrent_stale_registrations_return_one_workspace() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("registry.sqlite3");
        let mut production = HostRegistry::open(&path).unwrap();
        let host_id = production.ensure_local_host("local").unwrap();
        let development = HostRegistry::open(&path).unwrap();

        // Both channels make the same stale startup decision before either
        // write begins, matching bp and bp-dev launched in the same folder.
        assert!(production.snapshot().unwrap().workspaces.is_empty());
        assert!(development.snapshot().unwrap().workspaces.is_empty());
        let production_workspace = WorkspaceRecord::new(host_id, "/srv/shared");
        let development_workspace = WorkspaceRecord::new(host_id, "/srv/shared");
        assert_ne!(production_workspace.id, development_workspace.id);

        let start = Arc::new(Barrier::new(3));
        let production_start = Arc::clone(&start);
        let production = std::thread::spawn(move || {
            production_start.wait();
            production
                .insert_workspace_or_existing(&production_workspace)
                .unwrap()
        });
        let development_start = Arc::clone(&start);
        let development = std::thread::spawn(move || {
            development_start.wait();
            development
                .insert_workspace_or_existing(&development_workspace)
                .unwrap()
        });
        start.wait();

        let production = production.join().unwrap();
        let development = development.join().unwrap();
        assert_eq!(production, development);

        let registry = HostRegistry::open(&path).unwrap();
        assert_eq!(registry.snapshot().unwrap().workspaces, vec![production]);
    }
}
