use super::AgentEventStoreError;
use fs2::FileExt;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub(super) fn create_private_file(path: &Path) -> Result<(), AgentEventStoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let mut options = fs::OpenOptions::new();
    options.create(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

pub(super) fn secure_sqlite_files(path: &Path) -> Result<(), AgentEventStoreError> {
    #[cfg(unix)]
    for candidate in [
        path.to_owned(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ] {
        if candidate.try_exists()? {
            fs::set_permissions(candidate, fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(())
}

pub(super) fn lock_initialization(
    path: &Path,
) -> Result<AgentStoreInitializationLock, AgentEventStoreError> {
    let mut value = OsString::from(path.as_os_str());
    value.push(".init.lock");
    let lock_path = PathBuf::from(value);
    let mut options = fs::OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(&lock_path)?;
    #[cfg(unix)]
    fs::set_permissions(lock_path, fs::Permissions::from_mode(0o600))?;
    file.lock_exclusive()?;
    Ok(AgentStoreInitializationLock { _file: file })
}

pub(super) struct AgentStoreInitializationLock {
    _file: fs::File,
}
