use std::collections::BTreeSet;

use crate::transport::{HostCommand, HostTransport};

use super::super::model::{checked, ZellijError};
use super::validation::validate_name;
use super::{ZellijRuntime, LAUNCHER_PROGRAM, METADATA_TIMEOUT};

mod environment;

#[cfg(test)]
pub(super) use environment::candidate_directories_for_test;
use environment::{namespace_environment, validated_directory};

impl ZellijRuntime {
    /// Select the one socket namespace containing this exact live session.
    ///
    /// Before Blackpepper owned this choice, Zellij could create the same
    /// user's sessions under `/tmp`, the host temporary directory, an XDG
    /// runtime directory, or an explicit native socket root. Probe only roots
    /// that contain this session's Unix socket, and fail closed if two roots
    /// claim the same stable session ID. When none exists, new sessions use
    /// the short canonical `/tmp` root.
    pub fn resolve_session_namespace(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
    ) -> Result<(Self, bool), ZellijError> {
        validate_name("session", session)?;
        let environment = namespace_environment(host)?;
        let candidates = environment.candidates(crate::IS_DEVELOPMENT_BUILD)?;
        let mut active = Vec::new();
        let mut probed = BTreeSet::new();
        for candidate in candidates {
            let Some(candidate) = plausible_socket_directory(host, &candidate, session)? else {
                continue;
            };
            // `TMPDIR`, XDG, and an inherited native override can spell the
            // same directory through symlinks. Count one physical server once
            // so aliases never become a false ambiguity.
            if !probed.insert(candidate.clone()) {
                continue;
            }
            let runtime = self.with_socket_directory(candidate);
            if runtime.session_is_active(host, session)? {
                active.push(runtime);
            }
        }
        match active.len() {
            0 => Ok((
                self.with_socket_directory(
                    environment.creation_directory(crate::IS_DEVELOPMENT_BUILD)?,
                ),
                false,
            )),
            1 => Ok((active.pop().expect("one active namespace"), true)),
            _ => Err(ZellijError::AmbiguousSessionNamespace {
                session: session.to_owned(),
                socket_directories: active
                    .into_iter()
                    .filter_map(|runtime| runtime.socket_directory)
                    .collect(),
            }),
        }
    }

    fn with_socket_directory(&self, socket_directory: String) -> Self {
        Self {
            binary: self.binary.clone(),
            expected_version: self.expected_version.clone(),
            socket_directory: Some(socket_directory),
            config_file: self.config_file.clone(),
        }
    }
}

fn plausible_socket_directory(
    host: &mut dyn HostTransport,
    directory: &str,
    session: &str,
) -> Result<Option<String>, ZellijError> {
    let command = HostCommand::new(LAUNCHER_PROGRAM).args([
        "-c",
        "test -S \"$1/contract_version_1/$2\" 2>/dev/null || exit 3; cd -P -- \"$1\" || exit 4; pwd -P",
        "blackpepper-zellij-socket",
        directory,
        session,
    ]);
    let output = host.exec_timeout(&command, METADATA_TIMEOUT)?;
    if output.status == Some(3) && output.stdout.is_empty() && output.stderr.is_empty() {
        return Ok(None);
    }
    let output = checked(output, "validate Zellij socket namespace")?;
    let physical = String::from_utf8(output.stdout).map_err(|_| {
        ZellijError::InvalidOutput("The Zellij socket namespace was not valid UTF-8.".to_owned())
    })?;
    let physical = physical.trim_end_matches(['\n', '\r']);
    validated_directory("Zellij socket namespace", physical).map(Some)
}
