use std::path::Path;
use std::time::Duration;

use portable_pty::PtySize;

use crate::transport::{HostCommand, HostTransport, PtyProcess};

use super::super::model::{
    checked, client_list_reports_missing_session, parse_clients, ClientOperation, ZellijClient,
    ZellijError,
};
use super::metadata::read_output;
use super::validation::validate_name;
use super::ZellijRuntime;

const CLIENT_LIST_TIMEOUT: Duration = Duration::from_secs(2);

impl ZellijRuntime {
    pub fn list_clients_command(&self, session: &str) -> Result<HostCommand, ZellijError> {
        validate_name("session", session)?;
        Ok(self.session_action(session, ["list-clients"]))
    }

    pub fn list_clients(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
    ) -> Result<Vec<ZellijClient>, ZellijError> {
        self.list_clients_with_timeout(host, session, CLIENT_LIST_TIMEOUT)
    }

    pub(in crate::zellij::runtime) fn list_clients_with_timeout(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        timeout: Duration,
    ) -> Result<Vec<ZellijClient>, ZellijError> {
        let output = checked(
            self.list_clients_output_with_timeout(host, session, timeout)?,
            "list Zellij clients",
        )?;
        parse_clients(&String::from_utf8_lossy(&output.stdout))
    }

    fn list_clients_output(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
    ) -> Result<crate::transport::CommandOutput, ZellijError> {
        self.list_clients_output_with_timeout(host, session, CLIENT_LIST_TIMEOUT)
    }

    fn list_clients_output_with_timeout(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        timeout: Duration,
    ) -> Result<crate::transport::CommandOutput, ZellijError> {
        read_output(
            host,
            &self.list_clients_command(session)?,
            timeout,
            |output| {
                (output.success
                    && output.stderr.is_empty()
                    && output.stdout.iter().all(u8::is_ascii_whitespace))
                    || client_list_reports_missing_session(output, session)
            },
        )
    }

    /// Probe one exact session without consulting Zellij's resurrection list.
    ///
    /// `list-sessions --short` includes exited sessions from the shared cache
    /// but hides their EXITED marker, so it cannot establish that a server is
    /// active in the current socket namespace.
    pub fn session_is_active(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
    ) -> Result<bool, ZellijError> {
        let output = self.list_clients_output(host, session)?;
        if client_list_reports_missing_session(&output, session) {
            return Ok(false);
        }
        let output = checked(output, "probe Zellij session")?;
        parse_clients(&String::from_utf8_lossy(&output.stdout))?;
        Ok(true)
    }

    pub fn enforce_client_safety(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        operation: ClientOperation,
    ) -> Result<Vec<ZellijClient>, ZellijError> {
        let clients = self.list_clients(host, session)?;
        if !operation.allows(clients.len()) {
            return Err(ZellijError::ClientConflict { operation, clients });
        }
        Ok(clients)
    }

    pub fn attach(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        cwd: &Path,
        size: PtySize,
    ) -> Result<(PtyProcess, Vec<ZellijClient>), ZellijError> {
        // The shared read retries Zellij's transient recycled-client race.
        // Attach alone maps a final exact no-session response to the specific
        // pre-attach error; other callers retain their ordinary semantics.
        let output = self.list_clients_output(host, session)?;
        if client_list_reports_missing_session(&output, session) {
            return Err(ZellijError::SessionMissingBeforeAttach);
        }
        let output = checked(output, "list Zellij clients")?;
        let clients = parse_clients(&String::from_utf8_lossy(&output.stdout))?;
        let command = self.attach_command(session, cwd)?;
        Ok((host.attach_pty(&command, size)?, clients))
    }

    pub(crate) fn attach_command(
        &self,
        session: &str,
        cwd: &Path,
    ) -> Result<HostCommand, ZellijError> {
        validate_name("session", session)?;
        // This is a client-local override: it does not alter the user's
        // configuration or the session, but makes SIGTERM/SIGHUP detach this
        // attachment even when the user configured `on_force_close "quit"`.
        Ok(self
            .command(["attach", session, "options", "--on-force-close", "detach"])
            .cwd(cwd))
    }
}
