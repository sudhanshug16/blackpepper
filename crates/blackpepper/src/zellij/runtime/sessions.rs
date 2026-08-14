use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::transport::{HostCommand, HostTransport};

use super::super::model::{checked, parse_sessions, ClientOperation, ZellijError};
use super::validation::validate_name;
use super::{ZellijRuntime, DEVELOPMENT_SOCKET_OVERRIDE, METADATA_TIMEOUT};

const SESSION_EXIT_TIMEOUT: Duration = Duration::from_secs(5);
const SESSION_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(25);
const SESSION_KILL_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

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
        validate_name("session", session)?;
        if self.session_is_active(host, session)? {
            return Ok(false);
        }
        checked(
            host.exec(&self.create_session_with_env_command(session, cwd, env)?)?,
            "create Zellij session",
        )?;
        Ok(true)
    }

    pub fn kill_session(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
    ) -> Result<(), ZellijError> {
        self.kill_session_with_timeout(
            host,
            session,
            SESSION_EXIT_TIMEOUT,
            SESSION_EXIT_POLL_INTERVAL,
        )
    }

    fn kill_session_with_timeout(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<(), ZellijError> {
        self.enforce_client_safety(host, session, ClientOperation::Destroy)?;
        let command = self.command(["kill-session", session]);
        // A timed-out kill leaves the registry non-exited. The next leased
        // operation therefore reconciles this same deterministic session name
        // instead of assuming an unknown mutation succeeded.
        checked(
            host.exec_timeout(&command, SESSION_KILL_COMMAND_TIMEOUT)?,
            "kill Zellij session",
        )?;

        // On Unix the Zellij CLI only sends KillSession IPC before exiting.
        // Do not let callers reuse this deterministic name until the old
        // server has stopped accepting exact-session client queries.
        let deadline = Instant::now() + timeout;
        loop {
            if !self.session_is_active(host, session)? {
                return Ok(());
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(ZellijError::InvalidOutput(format!(
                    "Zellij session {session:?} remained active for {}ms after kill-session; refusing to mark it exited",
                    timeout.as_millis()
                )));
            }
            std::thread::sleep(poll_interval.min(deadline.saturating_duration_since(now)));
        }
    }

    #[cfg(test)]
    pub(crate) fn kill_session_with_timeout_for_test(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<(), ZellijError> {
        self.kill_session_with_timeout(host, session, timeout, poll_interval)
    }
}
