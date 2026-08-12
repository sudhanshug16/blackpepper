use std::time::Duration;

use crate::transport::{HostCommand, HostTransport, ZELLIJ_VERSION};

use super::model::{checked, ZellijError};

mod clients;
mod namespace;
#[cfg(test)]
mod namespace_tests;
mod panes;
mod sessions;
mod tabs;
mod validation;

pub const PINNED_VERSION: &str = ZELLIJ_VERSION;
pub(super) const METADATA_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const LAUNCHER_PROGRAM: &str = "/bin/sh";
pub(super) const LAUNCHER_ARG_ZERO: &str = "blackpepper-zellij";
pub(super) const DEVELOPMENT_SOCKET_OVERRIDE: &str = "BLACKPEPPER_E2E_ZELLIJ_SOCKET_DIR";
pub(super) const PROD_LAUNCHER_SCRIPT: &str = "bp_zellij_uid=$(id -u) || exit; ZELLIJ_SOCKET_DIR=/tmp/zellij-$bp_zellij_uid; export ZELLIJ_SOCKET_DIR; exec \"$@\"";
pub(super) const DEV_LAUNCHER_SCRIPT: &str = "if [ -n \"${BLACKPEPPER_E2E_ZELLIJ_SOCKET_DIR:-}\" ]; then ZELLIJ_SOCKET_DIR=$BLACKPEPPER_E2E_ZELLIJ_SOCKET_DIR; else bp_zellij_uid=$(id -u) || exit; ZELLIJ_SOCKET_DIR=/tmp/zellij-$bp_zellij_uid; fi; export ZELLIJ_SOCKET_DIR; exec \"$@\"";

#[derive(Debug, Clone)]
pub struct ZellijRuntime {
    binary: String,
    expected_version: String,
    socket_directory: Option<String>,
}

impl ZellijRuntime {
    pub fn new(binary: impl Into<String>) -> Result<Self, ZellijError> {
        Self::for_version(binary, PINNED_VERSION)
    }

    /// Build a runtime for a retained sidecar used by an existing session.
    /// New sessions should continue to use [`ZellijRuntime::new`].
    pub fn for_version(
        binary: impl Into<String>,
        expected_version: impl Into<String>,
    ) -> Result<Self, ZellijError> {
        let binary = binary.into();
        if binary.trim().is_empty() || binary.contains('\0') {
            return Err(ZellijError::InvalidName(
                "Zellij binary path must be non-empty".to_string(),
            ));
        }
        let expected_version = expected_version.into();
        if expected_version.is_empty()
            || expected_version.len() > 64
            || !expected_version
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_digit)
            || !expected_version.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '+' | '_')
            })
        {
            return Err(ZellijError::InvalidName(
                "Zellij version must be a non-empty token of at most 64 characters".to_string(),
            ));
        }
        Ok(Self {
            binary,
            expected_version,
            socket_directory: None,
        })
    }

    pub fn binary(&self) -> &str {
        &self.binary
    }

    pub fn version_command(&self) -> HostCommand {
        self.command(["--version"])
    }

    pub fn check_version(&self, host: &mut dyn HostTransport) -> Result<(), ZellijError> {
        let output = checked(
            host.exec_timeout(&self.version_command(), METADATA_TIMEOUT)?,
            "read Zellij version",
        )?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let actual = stdout.split_whitespace().last().unwrap_or_default();
        if actual != self.expected_version {
            return Err(ZellijError::VersionMismatch {
                expected: self.expected_version.clone(),
                actual: actual.to_string(),
            });
        }
        Ok(())
    }

    pub fn check_configuration_command(&self) -> HostCommand {
        self.command(["setup", "--check"])
    }

    /// Ask Zellij to parse its effective user configuration without writing
    /// to it. Invalid configuration is reported before Blackpepper creates or
    /// attaches a session, while native keybindings and options stay owned by
    /// Zellij.
    pub fn check_configuration(&self, host: &mut dyn HostTransport) -> Result<(), ZellijError> {
        checked(
            host.exec_timeout(&self.check_configuration_command(), METADATA_TIMEOUT)?,
            "check Zellij configuration",
        )?;
        Ok(())
    }

    pub(super) fn session_action<const N: usize>(
        &self,
        session: &str,
        action: [&str; N],
    ) -> HostCommand {
        self.command(["--session", session, "action"]).args(action)
    }

    /// Launch namespace-independent Zellij metadata commands through one
    /// host-side namespace wrapper, and resolved session commands directly.
    ///
    /// Zellij otherwise changes its socket root when `XDG_RUNTIME_DIR` is
    /// present, which can split desktop, SSH, and browser clients for the same
    /// Unix user. Native `ZELLIJ_SOCKET_DIR` is deliberately overridden. The
    /// internal E2E override is separate so test isolation cannot reintroduce
    /// this production split. The `/tmp` root also avoids Unix socket
    /// path-length failures. Once a session namespace is resolved, invoking
    /// the exact binary directly prevents workspace `PATH` or `BASH_ENV`
    /// values from influencing a shell launcher.
    pub(super) fn command<I, S>(&self, arguments: I) -> HostCommand
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        if let Some(socket_directory) = &self.socket_directory {
            return HostCommand::new(&self.binary)
                .env("ZELLIJ_SOCKET_DIR", socket_directory)
                .args(arguments);
        }
        HostCommand::new(LAUNCHER_PROGRAM)
            .args([
                "-c",
                launcher_script(),
                LAUNCHER_ARG_ZERO,
                self.binary.as_str(),
            ])
            .args(arguments)
    }
}

fn launcher_script() -> &'static str {
    if crate::IS_DEVELOPMENT_BUILD {
        DEV_LAUNCHER_SCRIPT
    } else {
        PROD_LAUNCHER_SCRIPT
    }
}
