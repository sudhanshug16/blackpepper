use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::transport::{HostCommand, HostTransport};

use super::super::model::{checked, parse_sessions, ClientOperation, ZellijError};
use super::validation::validate_name;
use super::{ZellijRuntime, DEVELOPMENT_SOCKET_OVERRIDE, METADATA_TIMEOUT};

const SESSION_READY_TIMEOUT: Duration = Duration::from_secs(5);
const SESSION_READY_POLL: Duration = Duration::from_millis(25);

impl ZellijRuntime {
    pub fn list_sessions_command(&self) -> HostCommand {
        self.command(["list-sessions", "--short", "--no-formatting"])
    }

    pub fn list_sessions(&self, host: &mut dyn HostTransport) -> Result<Vec<String>, ZellijError> {
        parse_sessions(host.exec_timeout(&self.list_sessions_command(), METADATA_TIMEOUT)?)
    }

    pub fn create_session_command(
        &self,
        session: &str,
        cwd: &Path,
    ) -> Result<HostCommand, ZellijError> {
        self.create_session_with_env_command(session, cwd, &BTreeMap::new())
    }

    pub fn create_session_with_env_command(
        &self,
        session: &str,
        cwd: &Path,
        env: &BTreeMap<String, String>,
    ) -> Result<HostCommand, ZellijError> {
        validate_name("session", session)?;
        let mut command = self
            .command(["attach", "--create-background", "--forget", session])
            .cwd(cwd);
        command.env.clone_from(env);
        // This internal override is inherited only by development E2E
        // harnesses. A project config must never redirect a real workspace's
        // Zellij server, even in a development client.
        command.env.remove(DEVELOPMENT_SOCKET_OVERRIDE);
        // A resolved runtime invokes the exact binary directly. Restore its
        // trusted socket root after copying workspace variables so a project
        // cannot redirect Blackpepper into a different Zellij namespace.
        if let Some(socket_directory) = &self.socket_directory {
            command
                .env
                .insert("ZELLIJ_SOCKET_DIR".to_owned(), socket_directory.clone());
        }
        Ok(command)
    }

    pub fn ensure_session(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        cwd: &Path,
    ) -> Result<bool, ZellijError> {
        self.ensure_session_with_env(host, session, cwd, &BTreeMap::new())
    }

    /// Create a session whose initial shell inherits workspace variables.
    /// Existing sessions are detected before the mutation, so reconnecting
    /// never changes the environment of a running session.
    pub fn ensure_session_with_env(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        cwd: &Path,
        env: &BTreeMap<String, String>,
    ) -> Result<bool, ZellijError> {
        self.ensure_session_with_env_and_timeout(host, session, cwd, env, SESSION_READY_TIMEOUT)
    }

    pub(crate) fn ensure_session_with_env_and_timeout(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        cwd: &Path,
        env: &BTreeMap<String, String>,
        readiness_timeout: Duration,
    ) -> Result<bool, ZellijError> {
        validate_name("session", session)?;
        if self.session_is_active(host, session)? {
            return Ok(false);
        }
        checked(
            host.exec(&self.create_session_with_env_command(session, cwd, env)?)?,
            "create Zellij session",
        )?;
        // `attach --create-background` can return before the new server is
        // ready for a client. Prove the exact session responds before handing
        // it to the PTY attach path; retry only the pinned missing-session
        // result and keep every other command failure fail-closed.
        let started = Instant::now();
        loop {
            if self.session_is_active(host, session)? {
                return Ok(true);
            }
            if started.elapsed() >= readiness_timeout {
                return Err(ZellijError::InvalidOutput(
                    "created Zellij session did not become ready before its bounded deadline"
                        .to_string(),
                ));
            }
            if crate::transport::CommandCancellation::scope_is_cancelled() {
                return Err(ZellijError::InvalidOutput(
                    "Zellij session creation was cancelled while waiting for readiness".to_string(),
                ));
            }
            std::thread::sleep(SESSION_READY_POLL);
        }
    }

    pub fn kill_session(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
    ) -> Result<(), ZellijError> {
        self.enforce_client_safety(host, session, ClientOperation::Destroy)?;
        let command = self.command(["kill-session", session]);
        checked(host.exec(&command)?, "kill Zellij session")?;
        Ok(())
    }
}
