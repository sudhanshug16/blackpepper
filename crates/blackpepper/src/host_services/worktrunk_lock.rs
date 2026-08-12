use super::process::run_bounded;
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs;
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

#[cfg(unix)]
#[path = "worktrunk_lock_guardian.rs"]
mod guardian;
#[cfg(unix)]
use guardian::LockGuardian;
#[cfg(unix)]
pub(super) use guardian::RegisteredProcessGroup;

/// Brief cross-process lock shared by every Blackpepper Worktrunk mutation.
pub(super) struct RepositoryLock {
    _file: fs::File,
    #[cfg(unix)]
    guardian: LockGuardian,
}

impl RepositoryLock {
    pub(super) fn acquire(lock_dir: &Path, repository: &Path) -> Result<Self, String> {
        fs::create_dir_all(lock_dir).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        fs::set_permissions(lock_dir, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
        let identity = repository_identity(repository);
        let digest = format!(
            "{:x}",
            Sha256::digest(identity.as_os_str().as_encoded_bytes())
        );
        let path = lock_dir.join(format!("{digest}.lock"));
        let mut options = fs::OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(&path).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
        file.try_lock_exclusive().map_err(|error| {
            format!("Another Worktrunk mutation is active for this repository: {error}")
        })?;
        #[cfg(unix)]
        let guardian = LockGuardian::spawn(file.as_raw_fd()).map_err(|error| {
            let _ = FileExt::unlock(&file);
            format!("Could not guard the Worktrunk repository lock: {error}")
        })?;
        Ok(Self {
            _file: file,
            #[cfg(unix)]
            guardian,
        })
    }

    /// Register a freshly-created Worktrunk process group before its gate is
    /// opened. The guardian owns an inherited copy of the repository lock, so
    /// a killed `bp-host` cannot release that lock until this group is gone.
    #[cfg(unix)]
    pub(super) fn register_process_group(
        &self,
        process_group: libc::pid_t,
    ) -> Result<RegisteredProcessGroup<'_>, String> {
        self.guardian.register(process_group)
    }

    #[cfg(all(test, unix))]
    pub(super) fn hold_guardian_lock_after_drop_for_test(&self) -> Result<(), String> {
        self.guardian.hold_lock_after_release_for_test()
    }
}

impl Drop for RepositoryLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        self.guardian.release();
        // Do not call LOCK_UN here. The guardian inherited this same open-file
        // description; an explicit unlock would also release its fail-closed
        // lock while a child group is still being contained. Closing this
        // process's descriptor naturally preserves the guardian's ownership.
    }
}

pub(super) fn repository_identity(repository: &Path) -> PathBuf {
    git_common_dir(repository).unwrap_or_else(|| repository.to_owned())
}

fn git_common_dir(repository: &Path) -> Option<PathBuf> {
    let output = run_bounded(
        OsStr::new("git"),
        [
            OsStr::new("-C"),
            repository.as_os_str(),
            OsStr::new("rev-parse"),
            OsStr::new("--path-format=absolute"),
            OsStr::new("--git-common-dir"),
        ],
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let path = PathBuf::from(path.trim());
    std::fs::canonicalize(&path).ok().or(Some(path))
}
