use std::path::Path;
use std::time::{Duration, Instant};

use crate::transport::{HostCommand, HostTransport};

use super::super::model::{checked, ClientOperation, ZellijError};
use super::validation::{path_text, validate_name, ValidateInitialCommand, BACKGROUND_TAB_LAYOUT};
use super::ZellijRuntime;

// Cancellation is briefly masked from new-tab through focus compensation, so
// each action needs its own hard bound. A timeout is an unknown remote result;
// callers surface it and a later list reconciles the deterministic tab name.
const BACKGROUND_MUTATION_TIMEOUT: Duration = Duration::from_secs(5);
const FOCUS_MUTATION_TIMEOUT: Duration = Duration::from_secs(5);
const FIRST_ATTACH_CLIENT_TIMEOUT: Duration = Duration::from_secs(2);
const FIRST_ATTACH_CLIENT_POLL: Duration = Duration::from_millis(25);

impl ZellijRuntime {
    pub fn new_tab_command(
        &self,
        session: &str,
        name: &str,
        cwd: &Path,
        initial_command: Option<&HostCommand>,
    ) -> Result<HostCommand, ZellijError> {
        validate_name("session", session)?;
        validate_name("tab", name)?;
        let cwd = path_text(cwd)?;
        // Zellij accepts `focus=false`, but 0.44.3 still moves its last-active
        // client to a dynamically created tab. `ensure_tab` therefore refuses
        // multiple clients and restores the sole client's previous tab.
        let mut command = self.session_action(
            session,
            [
                "new-tab",
                "--layout-string",
                BACKGROUND_TAB_LAYOUT,
                "--name",
                name,
                "--cwd",
                cwd.as_str(),
            ],
        );
        if let Some(initial) = initial_command {
            initial
                .validate_for_zellij()
                .map_err(ZellijError::InvalidName)?;
            command.args.push("--".to_string());
            command.args.push(initial.program.clone());
            command.args.extend(initial.args.iter().cloned());
        }
        Ok(command)
    }

    pub fn ensure_tab(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        name: &str,
        cwd: &Path,
        initial_command: Option<&HostCommand>,
    ) -> Result<(u64, bool), ZellijError> {
        let clients =
            self.enforce_client_safety(host, session, ClientOperation::BackgroundMutation)?;
        let tabs = self.list_tabs(host, session)?;
        if let Some(tab) = tabs.iter().find(|tab| tab.name == name) {
            return Ok((tab.tab_id, false));
        }
        let restore_tab_id = if clients.is_empty() {
            None
        } else {
            Some(
                tabs.iter()
                    .find(|tab| tab.active)
                    .ok_or_else(|| {
                        ZellijError::InvalidOutput(
                            "the attached Zellij client's active tab was not reported; refusing to create a focus-stealing tab"
                                .to_string(),
                        )
                    })?
                    .tab_id,
            )
        };
        crate::transport::CommandCancellation::mask_current(|| {
            let output = checked(
                host.exec_timeout(
                    &self.new_tab_command(session, name, cwd, initial_command)?,
                    BACKGROUND_MUTATION_TIMEOUT,
                )?,
                "create Zellij tab",
            )?;
            let id = String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse::<u64>()
                .map_err(|_| {
                    ZellijError::InvalidOutput("new-tab did not return a tab ID".to_string())
                })?;
            if let Some(tab_id) = restore_tab_id {
                let current_clients = self.list_clients(host, session)?;
                if current_clients.len() != 1
                    || current_clients[0].client_id != clients[0].client_id
                {
                    return Err(ZellijError::InvalidOutput(
                        "the Zellij client set changed while creating the tab; the tab was created, but Blackpepper cannot safely restore focus"
                            .to_string(),
                    ));
                }
                checked(
                    host.exec_timeout(
                        &self.focus_tab_command(session, tab_id)?,
                        BACKGROUND_MUTATION_TIMEOUT,
                    )?,
                    "restore Zellij client focus",
                )?;
            }
            Ok((id, true))
        })
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
        let tab_wait_started = Instant::now();
        let tabs = loop {
            match self.list_tabs_if_ready(host, session)? {
                Some(tabs) => break tabs,
                None if tab_wait_started.elapsed() >= client_wait_timeout => {
                    return Err(ZellijError::InvalidOutput(
                        "initial shell focus timed out waiting for Zellij tab metadata; no focus change was sent"
                            .to_string(),
                    ));
                }
                None if crate::transport::CommandCancellation::scope_is_cancelled() => {
                    return Err(ZellijError::InvalidOutput(
                        "initial shell focus was cancelled while waiting for Zellij tab metadata; no focus change was sent"
                            .to_string(),
                    ));
                }
                None => std::thread::sleep(FIRST_ATTACH_CLIENT_POLL),
            }
        };
        if !tabs.iter().any(|tab| tab.tab_id == INITIAL_SHELL_TAB_ID) {
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
}
