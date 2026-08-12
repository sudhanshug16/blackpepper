use crate::transport::{HostCommand, HostTransport};

use super::super::model::{
    checked, classify_pane_process, parse_panes, PaneProcessState, ZellijError, ZellijPane,
    ZellijTab,
};
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
        self.list_tabs_if_ready(host, session)?.ok_or_else(|| {
            ZellijError::InvalidOutput("list Zellij tabs returned no JSON".to_string())
        })
    }

    /// Zellij 0.44.3 has been observed returning success with empty stdout
    /// immediately after attach. Callers already in a bounded readiness loop
    /// may retry only this result; malformed non-empty output still fails.
    pub(super) fn list_tabs_if_ready(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
    ) -> Result<Option<Vec<ZellijTab>>, ZellijError> {
        let output = checked(
            host.exec_timeout(&self.list_tabs_command(session)?, METADATA_TIMEOUT)?,
            "list Zellij tabs",
        )?;
        if output.stdout.iter().all(u8::is_ascii_whitespace) {
            return Ok(None);
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|error| ZellijError::InvalidOutput(format!("invalid tab JSON: {error}")))
            .map(Some)
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
        let output = checked(
            host.exec_timeout(&self.list_panes_command(session)?, METADATA_TIMEOUT)?,
            "list Zellij panes",
        )?;
        parse_panes(&output.stdout)
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
