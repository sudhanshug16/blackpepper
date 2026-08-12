use fs2::FileExt;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub(super) fn lock_initialization(path: &Path) -> Result<ContextInitializationLock, String> {
    let mut value = OsString::from(path.as_os_str());
    value.push(".context.init.lock");
    let lock_path = PathBuf::from(value);
    let mut options = fs::OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(&lock_path)
        .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    fs::set_permissions(lock_path, fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    file.lock_exclusive().map_err(|error| error.to_string())?;
    Ok(ContextInitializationLock { _file: file })
}

pub(super) struct ContextInitializationLock {
    _file: fs::File,
}
