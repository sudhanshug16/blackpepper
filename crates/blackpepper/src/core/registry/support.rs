use crate::core::{
    paths::{create_private_dir, secure_existing_file},
    HostRecord, HostTransport, RepositoryIdentity, SessionRecord, WorkspaceRecord,
};
use fs2::FileExt;
use std::{
    error::Error,
    ffi::OsString,
    fmt, fs, io,
    os::raw::c_int,
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

pub(super) fn validate_host(host: &HostRecord) -> Result<(), RegistryError> {
    if host.display_name.trim().is_empty() {
        return Err(RegistryError::Validation(
            "host display name cannot be empty".to_owned(),
        ));
    }
    if matches!(&host.transport, HostTransport::Ssh { destination } if destination.trim().is_empty())
    {
        return Err(RegistryError::Validation(
            "SSH destination cannot be empty".to_owned(),
        ));
    }
    validate_timestamps(host.created_at_ms, host.updated_at_ms)
}

pub(super) fn validate_workspace(workspace: &WorkspaceRecord) -> Result<(), RegistryError> {
    if !Path::new(&workspace.root_path).is_absolute() {
        return Err(RegistryError::Validation(
            "workspace root must be absolute".to_owned(),
        ));
    }
    match &workspace.repository {
        Some(RepositoryIdentity::Local {
            host_id,
            git_common_dir,
        }) => {
            if *host_id != workspace.host_id {
                return Err(RegistryError::Validation(
                    "local repository identity must belong to the workspace host".to_owned(),
                ));
            }
            if !Path::new(git_common_dir).is_absolute() {
                return Err(RegistryError::Validation(
                    "Git common directory must be absolute".to_owned(),
                ));
            }
        }
        Some(RepositoryIdentity::Remote { canonical_url }) => {
            let Some((authority, path)) = canonical_url.split_once('/') else {
                return Err(invalid_remote_identity());
            };
            if authority.is_empty()
                || path.is_empty()
                || authority.contains('@')
                || authority != authority.to_ascii_lowercase()
                || canonical_url.contains('?')
                || canonical_url.contains('#')
                || canonical_url.contains("://")
                || canonical_url.ends_with(".git")
                || canonical_url.ends_with('/')
            {
                return Err(invalid_remote_identity());
            }
        }
        None => {}
    }
    validate_timestamps(workspace.created_at_ms, workspace.updated_at_ms)
}

fn invalid_remote_identity() -> RegistryError {
    RegistryError::Validation(
        "repository remote identity must be canonical and contain no credentials".to_owned(),
    )
}

pub(super) fn validate_session(session: &SessionRecord) -> Result<(), RegistryError> {
    if session.backend_version.trim().is_empty() {
        return Err(RegistryError::Validation(
            "backend version cannot be empty".to_owned(),
        ));
    }
    if session.backend_session_id.trim().is_empty() {
        return Err(RegistryError::Validation(
            "backend session ID cannot be empty".to_owned(),
        ));
    }
    validate_timestamps(session.created_at_ms, session.updated_at_ms)
}

fn validate_timestamps(created: i64, updated: i64) -> Result<(), RegistryError> {
    if created < 0 || updated < created {
        return Err(RegistryError::Validation(
            "record timestamps are inconsistent".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn create_private_file(path: &Path) -> Result<(), RegistryError> {
    let parent = path.parent().ok_or_else(|| {
        RegistryError::Validation("registry path must have a parent directory".to_owned())
    })?;
    create_private_dir(parent)?;
    let mut options = fs::OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)?;
    secure_existing_file(path)?;
    Ok(())
}

/// Serializes only SQLite WAL/schema initialization. This is distinct from
/// the interactive-client singleton and is safe across transient helpers.
pub(super) fn lock_registry_initialization(
    path: &Path,
    interrupted: &mut dyn FnMut() -> bool,
) -> Result<RegistryInitializationLock, RegistryError> {
    let lock_path = sidecar_path(path, ".init.lock");
    let mut options = fs::OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(&lock_path)?;
    secure_existing_file(&lock_path)?;
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => break,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if interrupted() {
                    return Err(RegistryError::Interrupted(
                        "registry initialization was cancelled".to_owned(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(RegistryInitializationLock { _file: file })
}

pub(super) struct RegistryInitializationLock {
    _file: fs::File,
}

pub(super) fn secure_sqlite_files(path: &Path) -> Result<(), RegistryError> {
    secure_existing_file(path)?;
    secure_existing_file(&sidecar_path(path, "-wal"))?;
    secure_existing_file(&sidecar_path(path, "-shm"))?;
    Ok(())
}

/// Keep WAL and shared-memory files at stable inodes across transient helper
/// and worker connections. Without this flag, the last connection that
/// SQLite considers active may unlink them while another idle connection in
/// the client still has the old generation open. A later worker can then open
/// a new zero-length WAL and hit `SQLITE_IOERR_SHORT_READ`.
pub(super) fn enable_persistent_wal(
    connection: &rusqlite::Connection,
) -> Result<(), RegistryError> {
    let mut enabled: c_int = 1;
    // SAFETY: `connection` exclusively owns this SQLite handle for the call,
    // `MAIN_DB` and the integer pointer remain valid for its duration, and the
    // documented PERSIST_WAL operation only reads/writes that integer.
    let status = unsafe {
        rusqlite::ffi::sqlite3_file_control(
            connection.handle(),
            rusqlite::MAIN_DB.as_ptr(),
            rusqlite::ffi::SQLITE_FCNTL_PERSIST_WAL,
            std::ptr::from_mut(&mut enabled).cast(),
        )
    };
    if status != rusqlite::ffi::SQLITE_OK {
        return Err(RegistryError::Sqlite(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(status),
            Some("could not enable persistent WAL files".to_owned()),
        )));
    }
    Ok(())
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = OsString::from(path.as_os_str());
    value.push(suffix);
    PathBuf::from(value)
}

#[derive(Debug)]
pub enum RegistryError {
    Io(io::Error),
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    Validation(String),
    UnsupportedSchema { found: u32, supported: u32 },
    UnexpectedValue(String),
    Interrupted(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "registry filesystem error: {error}"),
            Self::Sqlite(error) => {
                write!(formatter, "registry database error: {error}")?;
                if let rusqlite::Error::SqliteFailure(failure, _) = error {
                    write!(
                        formatter,
                        " (SQLite extended code {})",
                        failure.extended_code
                    )?;
                }
                Ok(())
            }
            Self::Json(error) => write!(formatter, "registry JSON error: {error}"),
            Self::Validation(message)
            | Self::UnexpectedValue(message)
            | Self::Interrupted(message) => formatter.write_str(message),
            Self::UnsupportedSchema { found, supported } => write!(
                formatter,
                "registry schema {found} is newer than supported schema {supported}"
            ),
        }
    }
}

impl Error for RegistryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for RegistryError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for RegistryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<serde_json::Error> for RegistryError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
