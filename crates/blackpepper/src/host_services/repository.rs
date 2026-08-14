use crate::core::{HostRegistry, RepositoryInspection, WorkspaceRecord};
use crate::workspace_identity::detect_local;
use std::path::{Path, PathBuf};

const MAX_PATH_BYTES: usize = 16 * 1024;
const MAX_DISPLAY_NAME_BYTES: usize = 512;

pub(super) fn inspect(
    registry: &HostRegistry,
    root_path: &str,
) -> Result<Option<RepositoryInspection>, String> {
    let root = canonical_workspace_root(root_path)?;
    let host_id = registry
        .local_host_id()
        .map_err(|error| error.to_string())?;
    detect_local(&root, host_id)
        .map(|detected| {
            detected.map(|repository| RepositoryInspection {
                identity: repository.identity,
                git_common_dir: repository.git_common_dir,
            })
        })
        .map_err(|error| format!("Could not inspect repository: {error}"))
}

pub(super) fn register(
    registry: &HostRegistry,
    root_path: &str,
    display_name: Option<String>,
) -> Result<WorkspaceRecord, String> {
    if display_name
        .as_ref()
        .is_some_and(|value| value.len() > MAX_DISPLAY_NAME_BYTES || value.contains('\0'))
    {
        return Err("Workspace display name is invalid or too long.".to_owned());
    }
    let root = canonical_workspace_root(root_path)?;
    let root_text = root
        .to_str()
        .ok_or_else(|| "Workspace path must be valid UTF-8.".to_owned())?
        .to_owned();
    let host_id = registry
        .local_host_id()
        .map_err(|error| error.to_string())?;
    let existing = registry
        .snapshot()
        .map_err(|error| error.to_string())?
        .workspaces
        .into_iter()
        .find(|workspace| workspace.host_id == host_id && workspace.root_path == root_text);
    let new_registration = existing.is_none();
    let mut workspace = existing.unwrap_or_else(|| WorkspaceRecord::new(host_id, &root_text));
    let detected = detect_local(&root, host_id)
        .map_err(|error| format!("Could not inspect repository: {error}"))?;
    workspace.repository = detected.map(|repository| repository.identity);
    if display_name.is_some() {
        workspace.display_name = display_name;
    }
    workspace.touch();
    if new_registration {
        workspace = registry
            .insert_workspace_or_existing(&workspace)
            .map_err(|error| error.to_string())?;
    } else {
        registry
            .upsert_workspace(&workspace)
            .map_err(|error| error.to_string())?;
    }
    Ok(workspace)
}

pub(super) fn canonical_workspace_root(value: &str) -> Result<PathBuf, String> {
    if value.is_empty() || value.len() > MAX_PATH_BYTES || value.contains('\0') {
        return Err("Workspace path is empty, invalid, or too long.".to_owned());
    }
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err("Workspace path must be absolute.".to_owned());
    }
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("Could not open workspace {}: {error}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!("Workspace {} is not a directory.", path.display()));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{CorePaths, HostRegistry};

    #[test]
    fn registration_is_idempotent_for_the_canonical_folder() {
        let root = tempfile::tempdir().unwrap();
        let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
        paths.prepare().unwrap();
        let mut registry = HostRegistry::open(paths.registry_path()).unwrap();
        registry.ensure_local_host("test").unwrap();
        let folder = root.path().join("workspace");
        std::fs::create_dir(&folder).unwrap();

        let first = register(&registry, folder.to_str().unwrap(), None).unwrap();
        let second = register(
            &registry,
            folder.to_str().unwrap(),
            Some("Named".to_owned()),
        )
        .unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.display_name.as_deref(), Some("Named"));
    }

    #[test]
    fn relative_paths_are_rejected() {
        assert!(canonical_workspace_root("relative/path").is_err());
    }
}
