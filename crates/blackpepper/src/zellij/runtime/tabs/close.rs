use crate::transport::{HostCommand, HostTransport};

use super::super::super::model::{checked, ClientOperation, ZellijError};
use super::super::validation::validate_name;
use super::super::ZellijRuntime;

impl ZellijRuntime {
    pub fn close_tab_command(
        &self,
        session: &str,
        tab_id: u64,
    ) -> Result<HostCommand, ZellijError> {
        validate_name("session", session)?;
        Ok(self.session_action(session, ["close-tab-by-id", &tab_id.to_string()]))
    }

    /// Closing a visible tab may move attached clients, so refuse it while
    /// multiple clients could be affected by Zellij's last-active routing.
    pub fn close_tab(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        tab_id: u64,
    ) -> Result<(), ZellijError> {
        self.enforce_client_safety(host, session, ClientOperation::FocusChange)?;
        checked(
            host.exec(&self.close_tab_command(session, tab_id)?)?,
            "close Zellij tab",
        )?;
        Ok(())
    }

    /// Close only while one exact terminal pane still carries the caller's
    /// immutable launch identity. Zellij can reuse numeric tab and pane IDs
    /// after native-client mutations, so IDs alone are never cleanup proof.
    pub fn close_tab_if_pane_matches(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        tab_id: u64,
        tab_name: &str,
        pane_selector: &str,
        expected_command_argument: &str,
    ) -> Result<bool, ZellijError> {
        self.enforce_client_safety(host, session, ClientOperation::FocusChange)?;
        let pane = self.terminal_pane_for_tab(host, session, tab_id)?;
        if pane.tab_name != tab_name
            || pane.selector() != pane_selector
            || !pane.has_command_argument(expected_command_argument)
        {
            return Ok(false);
        }
        checked(
            host.exec(&self.close_tab_command(session, tab_id)?)?,
            "close verified Zellij tab",
        )?;
        Ok(true)
    }
}
