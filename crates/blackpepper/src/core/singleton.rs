use super::paths::{create_private_dir, secure_existing_file};
use std::{
    error::Error,
    fmt, fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};

/// An advisory process lock. The file contents are diagnostic; the OS lock is authoritative.
#[derive(Debug)]
pub struct SingletonLock {
    file: fs::File,
    path: PathBuf,
    pid: u32,
}

impl SingletonLock {
    pub fn acquire(path: impl AsRef<Path>) -> Result<Self, SingletonLockError> {
        let path = path.as_ref().to_owned();
        let parent = path.parent().ok_or_else(|| {
            SingletonLockError::InvalidPath("lock path must have a parent directory".to_owned())
        })?;
        create_private_dir(parent)?;

        let mut options = fs::OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&path)?;
        secure_existing_file(&path)?;

        match try_lock_exclusive(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                let owner_pid = read_pid(&mut file).ok();
                return Err(SingletonLockError::AlreadyRunning {
                    path,
                    pid: owner_pid,
                });
            }
            Err(error) => return Err(SingletonLockError::Io(error)),
        }

        let pid = std::process::id();
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        writeln!(file, "{pid}")?;
        file.sync_data()?;
        Ok(Self { file, path, pid })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }
}

impl Drop for SingletonLock {
    fn drop(&mut self) {
        let _ = unlock(&self.file);
    }
}

fn read_pid(file: &mut fs::File) -> Result<u32, io::Error> {
    file.seek(SeekFrom::Start(0))?;
    let mut value = String::new();
    file.read_to_string(&mut value)?;
    value
        .trim()
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(unix)]
fn try_lock_exclusive(file: &fs::File) -> Result<(), io::Error> {
    // SAFETY: flock only reads the valid descriptor and does not retain a pointer.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn try_lock_exclusive(_file: &fs::File) -> Result<(), io::Error> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "advisory singleton locks are only implemented on Unix",
    ))
}

#[cfg(unix)]
fn unlock(file: &fs::File) -> Result<(), io::Error> {
    // SAFETY: flock only reads the valid descriptor and does not retain a pointer.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn unlock(_file: &fs::File) -> Result<(), io::Error> {
    Ok(())
}

#[derive(Debug)]
pub enum SingletonLockError {
    AlreadyRunning { path: PathBuf, pid: Option<u32> },
    InvalidPath(String),
    Io(io::Error),
}

impl fmt::Display for SingletonLockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning {
                path,
                pid: Some(pid),
            } => write!(
                formatter,
                "Blackpepper is already running as PID {pid}; terminate it before starting another client (lock: {})",
                path.display()
            ),
            Self::AlreadyRunning { path, pid: None } => write!(
                formatter,
                "Blackpepper is already running; terminate the existing process before starting another client (lock: {})",
                path.display()
            ),
            Self::InvalidPath(message) => formatter.write_str(message),
            Self::Io(error) => write!(formatter, "singleton lock error: {error}"),
        }
    }
}

impl Error for SingletonLockError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for SingletonLockError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn reports_the_owner_pid_and_releases_on_drop() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("run/bp.lock");
        let first = SingletonLock::acquire(&path).unwrap();
        let error = SingletonLock::acquire(&path).unwrap_err();
        assert!(matches!(
            error,
            SingletonLockError::AlreadyRunning { pid: Some(pid), .. }
                if pid == std::process::id()
        ));
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);

        drop(first);
        SingletonLock::acquire(&path).unwrap();
    }
}
