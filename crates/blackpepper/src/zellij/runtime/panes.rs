use crate::transport::{HostCommand, HostTransport};

use super::super::model::{
    classify_pane_process, PaneProcessState, ZellijError, ZellijPane, ZellijTab,
};
use super::metadata::read_json;
use super::validation::{validate_name, validate_pane_selector, validate_typed_pane_selector};
use super::{ZellijRuntime, METADATA_TIMEOUT};

impl ZellijRuntime {
    pub fn list_tabs_command(&self, session: &str) -> Result<HostCommand, ZellijError> {
        validate_name("session", session)?;
        Ok(self.session_action(session, ["list-tabs", "--json"]))
    }

    pub fn list_tabs(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
    ) -> Result<Vec<ZellijTab>, ZellijError> {
        self.list_tabs_with_timeout(host, session, METADATA_TIMEOUT)
    }

    pub(in crate::zellij::runtime) fn list_tabs_with_timeout(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        timeout: std::time::Duration,
    ) -> Result<Vec<ZellijTab>, ZellijError> {
        read_json(
            host,
            &self.list_tabs_command(session)?,
            "list Zellij tabs",
            "Timeout listing tabs",
            "tab",
            timeout,
        )
    }

    #[cfg(test)]
    pub(crate) fn list_tabs_with_timeout_for_test(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        timeout: std::time::Duration,
    ) -> Result<Vec<ZellijTab>, ZellijError> {
        self.list_tabs_with_timeout(host, session, timeout)
    }

    /// Build the long-lived host-local viewport subscription used by the
    /// blocker monitor. JSON mode emits one complete viewport per NDJSON line.
    pub fn subscribe_command(
        &self,
        session: &str,
        pane_id: &str,
    ) -> Result<HostCommand, ZellijError> {
        validate_name("session", session)?;
        validate_pane_selector(pane_id)?;
        Ok(self.command([
            "--session",
            session,
            "subscribe",
            "--pane-id",
            pane_id,
            "--format",
            "json",
        ]))
    }

    pub fn list_panes_command(&self, session: &str) -> Result<HostCommand, ZellijError> {
        validate_name("session", session)?;
        Ok(self.session_action(session, ["list-panes", "--json"]))
    }

    pub fn list_panes(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
    ) -> Result<Vec<ZellijPane>, ZellijError> {
        read_json(
            host,
            &self.list_panes_command(session)?,
            "list Zellij panes",
            "Timeout listing panes",
            "pane",
            METADATA_TIMEOUT,
        )
    }

    /// Observe a pane without attaching to or focusing its Zellij session.
    ///
    /// Zellij usually removes ordinary panes when their process exits. Command
    /// panes can instead remain as exited/held panes, so all three results are
    /// meaningful and must remain distinct.
    pub fn pane_process_state(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        tab_id: u64,
        tab_name: &str,
        pane_selector: &str,
        expected_command_argument: &str,
    ) -> Result<PaneProcessState, ZellijError> {
        validate_name("session", session)?;
        validate_typed_pane_selector(pane_selector)?;
        if expected_command_argument.is_empty()
            || expected_command_argument.len() > 256
            || expected_command_argument.chars().any(char::is_whitespace)
        {
            return Err(ZellijError::InvalidName(
                "Pane launch marker must be a bounded argument without whitespace".to_string(),
            ));
        }
        if !self.session_is_active(host, session)? {
            return Ok(PaneProcessState::Missing);
        }
        let panes = self.list_panes(host, session)?;
        Ok(classify_pane_process(
            &panes,
            tab_id,
            tab_name,
            pane_selector,
            expected_command_argument,
        ))
    }

    pub fn terminal_pane_for_tab(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        tab_id: u64,
    ) -> Result<ZellijPane, ZellijError> {
        let mut panes = self
            .list_panes(host, session)?
            .into_iter()
            .filter(|pane| pane.tab_id == tab_id && !pane.is_plugin);
        let pane = panes.next().ok_or_else(|| {
            ZellijError::InvalidOutput(format!("tab {tab_id} has no terminal pane"))
        })?;
        if panes.next().is_some() {
            return Err(ZellijError::InvalidOutput(format!(
                "tab {tab_id} has more than one terminal pane"
            )));
        }
        Ok(pane)
    }
}
