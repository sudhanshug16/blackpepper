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
    config_file: Option<String>,
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
            config_file: None,
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

    /// Select one Blackpepper-owned configuration without changing Zellij's
    /// native search path for runtimes that do not opt into it.
    pub fn with_config_file(mut self, path: impl Into<String>) -> Result<Self, ZellijError> {
        let path = path.into();
        if !std::path::Path::new(&path).is_absolute() || path.contains(['\0', '\n', '\r']) {
            return Err(ZellijError::InvalidName(
                "Zellij configuration path must be absolute".to_owned(),
            ));
        }
        self.config_file = Some(path);
        Ok(self)
    }

    /// Validate Zellij's native effective configuration and report whether
    /// the pinned binary found user or system configuration to own the UI.
    ///
    /// Zellij 0.44.3 has no machine-readable config-discovery API. Its pinned
    /// `setup --check` contract is therefore parsed narrowly and covered by
    /// exact-output tests. Any unfamiliar successful output fails closed.
    pub fn user_configuration_present(
        &self,
        host: &mut dyn HostTransport,
    ) -> Result<bool, ZellijError> {
        Ok(self.user_configuration(host)?.1)
    }

    /// The host's own configuration path, and whether a file exists there.
    /// Blackpepper merges its appearance into that file rather than replacing
    /// it, so the path matters even when the file is absent.
    pub fn user_configuration(
        &self,
        host: &mut dyn HostTransport,
    ) -> Result<(Option<String>, bool), ZellijError> {
        let output = checked(
            host.exec_timeout(&self.check_configuration_command(), METADATA_TIMEOUT)?,
            "check Zellij configuration",
        )?;
        configuration_source(&output.stdout, &output.stderr)
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
        let mut arguments = arguments.into_iter().map(Into::into).collect::<Vec<_>>();
        if let Some(config_file) = &self.config_file {
            arguments.splice(0..0, ["--config".to_owned(), config_file.clone()]);
        }
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

/// Where the pinned binary looks for the host's own configuration, and
/// whether a file is actually there. Zellij prints the path whether or not it
/// exists, so the two facts are reported together.
pub fn configuration_source(
    stdout: &[u8],
    stderr: &[u8],
) -> Result<(Option<String>, bool), ZellijError> {
    let present = configuration_present(stdout, stderr)?;
    let diagnostics = format!(
        "{}\n{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    );
    let path = diagnostics
        .lines()
        .find_map(|line| line.trim().strip_prefix("[LOOKING FOR CONFIG FILE FROM]:"))
        .map(|value| value.trim().trim_matches('"').to_owned())
        .filter(|value| !value.is_empty() && value != "Not Found");
    Ok((path, present))
}

fn configuration_present(stdout: &[u8], stderr: &[u8]) -> Result<bool, ZellijError> {
    let stdout = std::str::from_utf8(stdout).map_err(|_| {
        ZellijError::InvalidOutput("Zellij configuration diagnostics were not UTF-8.".to_owned())
    })?;
    let stderr = std::str::from_utf8(stderr).map_err(|_| {
        ZellijError::InvalidOutput("Zellij configuration diagnostics were not UTF-8.".to_owned())
    })?;
    let diagnostics = format!("{stdout}\n{stderr}");
    if diagnostics.contains("[CONFIG DIR]: Not Found")
        && diagnostics.contains("[CONFIG FILE]: Not Found")
    {
        return Ok(false);
    }
    if diagnostics.contains("[CONFIG FILE]: Well defined.")
        || diagnostics.contains("[LOOKING FOR CONFIG FILE FROM]:")
        || diagnostics.contains("[CONFIG ERROR]:")
    {
        return Ok(true);
    }
    Err(ZellijError::InvalidOutput(
        "Zellij returned unfamiliar configuration diagnostics.".to_owned(),
    ))
}

fn launcher_script() -> &'static str {
    if crate::IS_DEVELOPMENT_BUILD {
        DEV_LAUNCHER_SCRIPT
    } else {
        PROD_LAUNCHER_SCRIPT
    }
}
