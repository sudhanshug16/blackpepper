use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::Command;

use super::TransportError;

mod running;
mod running_wait;
pub use running::RunningCommand;

/// An argv-based command to run on a workspace host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
}

impl HostCommand {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
        }
    }

    pub fn arg(mut self, value: impl Into<String>) -> Self {
        self.args.push(value.into());
        self
    }

    pub fn args<I, S>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(values.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, value: impl Into<PathBuf>) -> Self {
        self.cwd = Some(value.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub(crate) fn validate(&self) -> Result<(), TransportError> {
        if self.program.is_empty() || self.program.contains('\0') {
            return Err(TransportError::InvalidCommand(
                "command program must be non-empty and contain no NUL bytes".to_string(),
            ));
        }
        if self.args.iter().any(|argument| argument.contains('\0')) {
            return Err(TransportError::InvalidCommand(
                "command arguments must not contain NUL bytes".to_string(),
            ));
        }
        for (key, value) in &self.env {
            if !valid_environment_key(key) || value.contains('\0') {
                return Err(TransportError::InvalidEnvironment(format!(
                    "invalid environment assignment for '{key}'"
                )));
            }
        }
        Ok(())
    }

    /// Build the command line sent to the Linux user's login shell by SSH.
    pub(crate) fn remote_shell_line(&self) -> Result<String, TransportError> {
        self.validate()?;
        let mut words = Vec::with_capacity(self.args.len() + self.env.len() + 2);
        if !self.env.is_empty() {
            words.push("env".to_string());
            words.extend(self.env.iter().map(|(key, value)| format!("{key}={value}")));
        }
        words.push(self.program.clone());
        words.extend(self.args.iter().cloned());

        let command = shell_words::join(words);
        match &self.cwd {
            Some(cwd) => {
                let cwd = cwd.to_str().ok_or_else(|| {
                    TransportError::InvalidCommand(
                        "remote working directory must be valid UTF-8".to_string(),
                    )
                })?;
                Ok(format!("cd {} && exec {command}", shell_words::quote(cwd)))
            }
            None => Ok(format!("exec {command}")),
        }
    }
}

fn valid_environment_key(key: &str) -> bool {
    let mut characters = key.chars();
    matches!(characters.next(), Some('_' | 'a'..='z' | 'A'..='Z'))
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

/// A concrete local process produced by a transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<OsString, OsString>,
    pub(crate) creation_umask: Option<u32>,
}

impl ProcessSpec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            creation_umask: None,
        }
    }

    pub fn arg(mut self, value: impl Into<OsString>) -> Self {
        self.args.push(value.into());
        self
    }

    pub fn args<I, S>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(values.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, value: impl Into<PathBuf>) -> Self {
        self.cwd = Some(value.into());
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub(crate) fn creation_umask(mut self, mask: u32) -> Self {
        self.creation_umask = Some(mask);
        self
    }

    pub fn argv(&self) -> impl Iterator<Item = &OsStr> {
        std::iter::once(self.program.as_os_str()).chain(self.args.iter().map(OsString::as_os_str))
    }

    pub(crate) fn to_command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        command.envs(&self.env);
        #[cfg(unix)]
        if let Some(mask) = self.creation_umask {
            use std::os::unix::process::CommandExt;

            // SAFETY: `pre_exec` runs after fork and before exec. `umask` is
            // async-signal-safe, has no failure mode, and touches no Rust state.
            unsafe {
                command.pre_exec(move || {
                    libc::umask(mask as libc::mode_t);
                    Ok(())
                });
            }
        }
        command
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl From<std::process::Output> for CommandOutput {
    fn from(output: std::process::Output) -> Self {
        Self {
            success: output.status.success(),
            status: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        }
    }
}

#[cfg(test)]
mod tests;
