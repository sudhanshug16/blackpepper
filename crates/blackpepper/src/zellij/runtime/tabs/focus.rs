use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crate::transport::{HostCommand, HostTransport};

use super::super::super::model::{checked, ZellijError};
use super::super::validation::validate_name;
use super::super::ZellijRuntime;

const FOCUS_MUTATION_TIMEOUT: Duration = Duration::from_secs(5);
const FIRST_ATTACH_CLIENT_TIMEOUT: Duration = Duration::from_secs(2);
const FIRST_ATTACH_CLIENT_POLL: Duration = Duration::from_millis(25);

impl ZellijRuntime {
    pub(super) fn restore_background_focus(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        created_name: &str,
        preexisting_tab_ids: &BTreeSet<u64>,
        restore_focus: Option<(u32, u64)>,
    ) -> Result<(), ZellijError> {
        let Some((client_id, restore_tab_id)) = restore_focus else {
            return Ok(());
        };
        // Creating the tab legitimately changes the client's focused pane and
        // therefore its full list-clients row. The numeric client ID is the
        // only stable server token across the mutation; sandwich the tab read
        // between two identical post-mutation rows to catch detach/navigation
        // churn before sending a focus command. Zellij can immediately reuse
        // a detached ID, so an identical native-client replacement remains
        // indistinguishable without a server-side connection generation.
        let clients_before = self.list_clients(host, session)?;
        match clients_before.as_slice() {
            [] => return Ok(()),
            [client] if client.client_id == client_id => {}
            _ => return Err(changed_background_client_error()),
        }
        let tabs = self.list_tabs(host, session)?;
        let clients_after = self.list_clients(host, session)?;
        if clients_after.is_empty() {
            return Ok(());
        }
        if clients_after != clients_before {
            return Err(changed_background_client_error());
        }
        let active_tabs = tabs.iter().filter(|tab| tab.active).collect::<Vec<_>>();
        let [active] = active_tabs.as_slice() else {
            return Err(ZellijError::InvalidOutput(format!(
                "Zellij reported {} active tabs for one client; Blackpepper cannot safely restore focus",
                active_tabs.len()
            )));
        };
        if active.tab_id == restore_tab_id
            || active.name != created_name
            || preexisting_tab_ids.contains(&active.tab_id)
        {
            return Ok(());
        }
        checked(
            host.exec_timeout(
                &self.focus_tab_command(session, restore_tab_id)?,
                FOCUS_MUTATION_TIMEOUT,
            )?,
            "restore Zellij client focus",
        )?;
        Ok(())
    }

    pub fn focus_tab_command(
        &self,
        session: &str,
        tab_id: u64,
    ) -> Result<HostCommand, ZellijError> {
        validate_name("session", session)?;
        Ok(self.session_action(session, ["go-to-tab-by-id", &tab_id.to_string()]))
    }

    /// Focus the initial workspace shell only while one stable client owns the
    /// session. The caller holds Blackpepper's workspace lifecycle lease, so
    /// another Blackpepper client cannot attach between validation and focus.
    /// Rechecking after tab discovery also refuses native-client races before
    /// the focus-changing command is sent.
    pub fn focus_initial_shell_for_single_client(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
    ) -> Result<(), ZellijError> {
        self.focus_initial_shell_for_single_client_with_timeout(
            host,
            session,
            FIRST_ATTACH_CLIENT_TIMEOUT,
        )
    }

    pub(crate) fn focus_initial_shell_for_single_client_with_timeout(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        client_wait_timeout: Duration,
    ) -> Result<(), ZellijError> {
        const INITIAL_SHELL_TAB_ID: u64 = 0;

        let started = Instant::now();
        let clients = loop {
            let clients = self.list_clients(host, session)?;
            match clients.len() {
                1 => break clients,
                count if count > 1 => {
                    return Err(ZellijError::InvalidOutput(format!(
                        "initial shell focus requires exactly 1 controlling client; found {count}; no focus change was sent"
                    )));
                }
                _ if started.elapsed() >= client_wait_timeout => {
                    return Err(ZellijError::InvalidOutput(
                        "initial shell focus timed out waiting for its controlling client; no focus change was sent"
                            .to_string(),
                    ));
                }
                _ if crate::transport::CommandCancellation::scope_is_cancelled() => {
                    return Err(ZellijError::InvalidOutput(
                        "initial shell focus was cancelled before its controlling client appeared; no focus change was sent"
                            .to_string(),
                    ));
                }
                _ => std::thread::sleep(FIRST_ATTACH_CLIENT_POLL),
            }
        };
        if !self
            .list_tabs(host, session)?
            .iter()
            .any(|tab| tab.tab_id == INITIAL_SHELL_TAB_ID)
        {
            return Err(ZellijError::InvalidOutput(
                "the workspace's initial shell tab (ID 0) is missing; no focus change was sent"
                    .to_string(),
            ));
        }
        let current_clients = self.list_clients(host, session)?;
        if current_clients != clients {
            return Err(ZellijError::InvalidOutput(
                "the Zellij client set changed while selecting the initial shell; no focus change was sent"
                    .to_string(),
            ));
        }
        crate::transport::CommandCancellation::mask_current(|| {
            checked(
                host.exec_timeout(
                    &self.focus_tab_command(session, INITIAL_SHELL_TAB_ID)?,
                    FOCUS_MUTATION_TIMEOUT,
                )?,
                "focus initial workspace shell",
            )?;
            Ok(())
        })
    }
}

fn changed_background_client_error() -> ZellijError {
    ZellijError::InvalidOutput(
        "the Zellij client set changed while creating the tab; Blackpepper cannot safely restore focus"
            .to_string(),
    )
}
