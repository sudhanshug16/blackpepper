use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use tempfile::{Builder as TempBuilder, TempDir};

use super::TransportError;

/// Conservative limit that fits both Linux and macOS Unix socket paths.
pub const CONTROL_PATH_MAX_BYTES: usize = 100;

#[derive(Debug, Clone)]
pub struct SshConfig {
    pub destination: String,
    pub ssh_binary: PathBuf,
    pub config_file: Option<PathBuf>,
    pub master_args: Vec<OsString>,
    pub control_root: Option<PathBuf>,
}

impl SshConfig {
    pub fn new(destination: impl Into<String>) -> Self {
        Self {
            destination: destination.into(),
            ssh_binary: PathBuf::from("ssh"),
            config_file: None,
            master_args: Vec::new(),
            control_root: None,
        }
    }

    pub fn validate(&self) -> Result<(), TransportError> {
        if self.destination.trim().is_empty()
            || self.destination.starts_with('-')
            || self.destination.contains(['\0', '\n', '\r'])
            || self.destination.chars().any(char::is_whitespace)
        {
            return Err(TransportError::InvalidSshConfiguration(
                "SSH destination must be a non-empty, single host argument".to_string(),
            ));
        }
        if self.ssh_binary.as_os_str().is_empty() {
            return Err(TransportError::InvalidSshConfiguration(
                "SSH executable path must be non-empty".to_string(),
            ));
        }
        for argument in &self.master_args {
            if let Some(argument) = argument.to_str() {
                if reserved_master_argument(argument) {
                    return Err(TransportError::InvalidSshConfiguration(format!(
                        "SSH master argument '{argument}' is owned by Blackpepper"
                    )));
                }
            }
        }
        Ok(())
    }
}

fn reserved_master_argument(argument: &str) -> bool {
    const RESERVED: &[&str] = &[
        "-D", "-F", "-f", "-L", "-M", "-N", "-n", "-O", "-R", "-S", "-T", "-t", "-W", "--",
    ];
    RESERVED
        .iter()
        .any(|flag| argument == *flag || (flag.len() == 2 && argument.starts_with(flag)))
}

/// Private directory and socket path owned by one control master.
pub struct ControlSocket {
    directory: TempDir,
    path: PathBuf,
}

impl ControlSocket {
    pub fn allocate(root: Option<&Path>) -> Result<Self, TransportError> {
        let mut builder = TempBuilder::new();
        builder.prefix("bp-ssh-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            // Set the private mode in the atomic mkdir itself. The standard
            // 0o022 umask would otherwise produce a searchable 0o755 directory.
            builder.permissions(fs::Permissions::from_mode(0o700));
        }
        let directory = match root {
            Some(root) => {
                fs::create_dir_all(root).map_err(|source| {
                    TransportError::io("failed to create SSH runtime root", source)
                })?;
                builder.tempdir_in(root).map_err(|source| {
                    TransportError::io("failed to create SSH runtime directory", source)
                })?
            }
            None => {
                let short_root = default_control_root();
                builder.tempdir_in(&short_root).map_err(|source| {
                    TransportError::io("failed to create SSH runtime directory", source)
                })?
            }
        };
        let path = directory.path().join("c");
        let length = path_bytes(&path);
        if length >= CONTROL_PATH_MAX_BYTES {
            return Err(TransportError::ControlPathTooLong {
                path: path.to_string_lossy().to_string(),
                max_bytes: CONTROL_PATH_MAX_BYTES,
            });
        }
        Ok(Self { directory, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn directory(&self) -> &Path {
        self.directory.path()
    }
}

#[cfg(unix)]
fn default_control_root() -> PathBuf {
    PathBuf::from("/tmp")
}

#[cfg(not(unix))]
fn default_control_root() -> PathBuf {
    std::env::temp_dir()
}

impl fmt::Debug for ControlSocket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControlSocket")
            .field("path", &self.path)
            .finish()
    }
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> usize {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().len()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> usize {
    path.to_string_lossy().len()
}
