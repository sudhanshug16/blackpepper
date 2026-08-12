use std::{
    env,
    error::Error,
    ffi::OsString,
    fmt, fs, io,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const APPLICATION_DIR: &str = "blackpepper";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CorePaths {
    state_dir: PathBuf,
    runtime_dir: PathBuf,
}

impl CorePaths {
    pub fn discover() -> Result<Self, PathError> {
        let state_root = absolute_env_path("XDG_STATE_HOME")?
            .or_else(dirs::state_dir)
            .ok_or(PathError::StateDirectoryUnavailable)?;
        let state_dir = state_root.join(APPLICATION_DIR);
        let runtime_dir = match absolute_env_path("XDG_RUNTIME_DIR")? {
            Some(root) => root.join(APPLICATION_DIR),
            None => state_dir.join("run"),
        };
        Ok(Self {
            state_dir,
            runtime_dir,
        })
    }

    /// Builds paths from XDG-like roots. Primarily useful for embedding and tests.
    pub fn from_roots(state_root: impl Into<PathBuf>, runtime_root: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_root.into().join(APPLICATION_DIR),
            runtime_dir: runtime_root.into().join(APPLICATION_DIR),
        }
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    pub fn registry_path(&self) -> PathBuf {
        self.state_dir.join("host-registry.sqlite3")
    }

    pub fn singleton_lock_path(&self) -> PathBuf {
        // A client launched from a desktop session may have XDG_RUNTIME_DIR while
        // one launched through SSH or an embedded terminal may not. Keep the
        // per-channel singleton in the state tree so both environments contend
        // on the same advisory lock. Production and development get one client
        // each. A source-watch run is a third, temporary channel. The stable
        // workspace/session registry and persistent safety locks remain shared
        // so every channel sees the same live resources.
        self.singleton_lock_path_for(client_channel())
    }

    fn singleton_lock_path_for(&self, channel: ClientChannel) -> PathBuf {
        self.state_dir.join(match channel {
            ClientChannel::Production => "run/bp.lock",
            ClientChannel::Development => "run/bp-dev.lock",
            ClientChannel::SourceWatch => "run/bp-watch.lock",
        })
    }

    pub fn agent_events_path(&self) -> PathBuf {
        self.agent_events_path_for(client_channel())
    }

    fn agent_events_path_for(&self, channel: ClientChannel) -> PathBuf {
        // Agent runs are launch/build-channel state, unlike the shared host,
        // workspace, and session inventory. Keeping the development event
        // schema separate prevents an experimental migration from making the
        // installed production helper unable to record or reconcile its own
        // live agents. The dev path is stable across rebuilds so compatible
        // upgrades can still rehydrate an existing development run.
        self.state_dir.join(match channel {
            ClientChannel::Production => "agent-events.sqlite3",
            ClientChannel::Development => "agent-events-dev.sqlite3",
            ClientChannel::SourceWatch => "agent-events-watch.sqlite3",
        })
    }

    pub fn repository_lock_dir(&self) -> PathBuf {
        // These coordination locks must remain identical when one client has
        // XDG_RUNTIME_DIR (desktop) and another does not (SSH/browser). They
        // are advisory, so persistent empty lock files are harmless.
        self.state_dir.join("run/repository-locks")
    }

    pub fn session_lock_dir(&self) -> PathBuf {
        self.state_dir.join("run/session-locks")
    }

    pub fn prepare(&self) -> Result<(), PathError> {
        create_private_dir(&self.state_dir)?;
        create_private_dir(&self.runtime_dir)?;
        create_private_dir(&self.state_dir.join("run"))?;
        create_private_dir(&self.repository_lock_dir())?;
        create_private_dir(&self.session_lock_dir())?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClientChannel {
    Production,
    Development,
    SourceWatch,
}

fn client_channel() -> ClientChannel {
    if crate::IS_SOURCE_WATCH_BUILD {
        ClientChannel::SourceWatch
    } else if crate::IS_DEVELOPMENT_BUILD {
        ClientChannel::Development
    } else {
        ClientChannel::Production
    }
}

fn absolute_env_path(name: &'static str) -> Result<Option<PathBuf>, PathError> {
    let Some(value) = env::var_os(name).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let path = PathBuf::from(&value);
    if !path.is_absolute() {
        return Err(PathError::RelativeEnvironmentPath { name, value });
    }
    Ok(Some(path))
}

pub(crate) fn create_private_dir(path: &Path) -> Result<(), io::Error> {
    fs::create_dir_all(path)?;
    set_mode(path, 0o700)
}

pub(crate) fn secure_existing_file(path: &Path) -> Result<(), io::Error> {
    if path.try_exists()? {
        set_mode(path, 0o600)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), io::Error> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), io::Error> {
    Ok(())
}

#[derive(Debug)]
pub enum PathError {
    RelativeEnvironmentPath { name: &'static str, value: OsString },
    StateDirectoryUnavailable,
    Io(io::Error),
}

impl fmt::Display for PathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativeEnvironmentPath { name, value } => write!(
                formatter,
                "{name} must be absolute, but was {}",
                Path::new(value).display()
            ),
            Self::StateDirectoryUnavailable => {
                formatter.write_str("could not determine an XDG state directory")
            }
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl Error for PathError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for PathError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::core::{SingletonLock, SingletonLockError};
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn prepares_private_xdg_directories() {
        let root = tempfile::tempdir().unwrap();
        let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
        paths.prepare().unwrap();

        assert_eq!(
            fs::metadata(paths.state_dir()).unwrap().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(paths.runtime_dir()).unwrap().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(paths.session_lock_dir()).unwrap().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(paths.singleton_lock_path().parent().unwrap())
                .unwrap()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn singleton_is_shared_across_different_runtime_roots() {
        let root = tempfile::tempdir().unwrap();
        let state_root = root.path().join("state");
        let desktop = CorePaths::from_roots(&state_root, root.path().join("desktop-run"));
        let embedded = CorePaths::from_roots(&state_root, root.path().join("fallback-run"));

        assert_eq!(
            desktop.singleton_lock_path(),
            embedded.singleton_lock_path()
        );
        assert_eq!(
            desktop.repository_lock_dir(),
            embedded.repository_lock_dir()
        );
        assert_eq!(desktop.session_lock_dir(), embedded.session_lock_dir());

        let first = SingletonLock::acquire(desktop.singleton_lock_path()).unwrap();
        let error = SingletonLock::acquire(embedded.singleton_lock_path()).unwrap_err();
        assert!(matches!(
            error,
            SingletonLockError::AlreadyRunning { pid: Some(pid), .. }
                if pid == std::process::id()
        ));
        drop(first);
    }

    #[test]
    fn runtime_channels_have_distinct_singletons_but_shared_state_locks() {
        let root = tempfile::tempdir().unwrap();
        let paths = CorePaths::from_roots(root.path().join("state"), root.path().join("run"));
        paths.prepare().unwrap();
        let production = paths.singleton_lock_path_for(ClientChannel::Production);
        let development = paths.singleton_lock_path_for(ClientChannel::Development);
        let source_watch = paths.singleton_lock_path_for(ClientChannel::SourceWatch);

        assert_ne!(production, development);
        assert_ne!(production, source_watch);
        assert_ne!(development, source_watch);
        let event_paths = [
            paths.agent_events_path_for(ClientChannel::Production),
            paths.agent_events_path_for(ClientChannel::Development),
            paths.agent_events_path_for(ClientChannel::SourceWatch),
        ];
        assert_ne!(event_paths[0], event_paths[1]);
        assert_ne!(event_paths[0], event_paths[2]);
        assert_ne!(event_paths[1], event_paths[2]);
        let production_lock = SingletonLock::acquire(&production).unwrap();
        let development_lock = SingletonLock::acquire(&development).unwrap();
        let source_watch_lock = SingletonLock::acquire(&source_watch).unwrap();
        assert_eq!(
            paths.registry_path(),
            paths.state_dir().join("host-registry.sqlite3")
        );
        assert_eq!(
            paths.repository_lock_dir(),
            paths.state_dir().join("run/repository-locks")
        );
        assert_eq!(
            paths.session_lock_dir(),
            paths.state_dir().join("run/session-locks")
        );
        drop((production_lock, development_lock, source_watch_lock));
    }
}
