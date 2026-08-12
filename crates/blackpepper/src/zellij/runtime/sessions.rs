use std::collections::BTreeMap;
use std::path::Path;

use crate::transport::{HostCommand, HostTransport};

use super::super::model::{checked, parse_sessions, ClientOperation, ZellijError};
use super::validation::validate_name;
use super::{ZellijRuntime, DEVELOPMENT_SOCKET_OVERRIDE, METADATA_TIMEOUT};

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
        self.enforce_client_safety(host, session, ClientOperation::Destroy)?;
        let command = self.command(["kill-session", session]);
        checked(host.exec(&command)?, "kill Zellij session")?;
        Ok(())
    }
}
