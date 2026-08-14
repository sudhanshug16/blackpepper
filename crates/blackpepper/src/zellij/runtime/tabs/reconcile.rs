use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crate::transport::{HostTransport, TransportError};

use super::super::super::model::{checked, parse_panes, ZellijError, ZellijPane, ZellijTab};
use super::super::metadata::transient_metadata_result;
use super::super::{ZellijRuntime, METADATA_TIMEOUT};

pub(super) const TAB_CREATION_RECONCILE_TIMEOUT: Duration = Duration::from_secs(5);
const TAB_CREATION_RECONCILE_POLL: Duration = Duration::from_millis(25);

pub(super) enum CreationReceipt {
    Reported(u64),
    Unknown(String),
}

impl CreationReceipt {
    pub(super) fn reported(tab_id: u64) -> Self {
        Self::Reported(tab_id)
    }

    pub(super) fn unknown(reason: impl Into<String>) -> Self {
        Self::Unknown(reason.into())
    }

    fn reported_id(&self) -> Option<u64> {
        match self {
            Self::Reported(tab_id) => Some(*tab_id),
            Self::Unknown(_) => None,
        }
    }

    fn ownership_confirmed(&self, tab_id: u64) -> bool {
        matches!(self, Self::Reported(reported) if *reported == tab_id)
    }

    fn failure_reason(&self) -> String {
        match self {
            Self::Reported(tab_id) => format!("new-tab reported tab ID {tab_id}"),
            Self::Unknown(reason) => reason.clone(),
        }
    }
}

impl ZellijRuntime {
    /// Zellij 0.44.3 can finish creating a tab after its action client exits
    /// successfully without an ID. Observe the one deterministic result and
    /// its pane instead of repeating an unknown mutation and creating a
    /// duplicate. A reported ID remains binding so a stale reply can never be
    /// replaced silently with a different same-name tab.
    pub(super) fn reconcile_created_tab(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        name: &str,
        preexisting_tab_ids: &BTreeSet<u64>,
        receipt: CreationReceipt,
        timeout: Duration,
    ) -> Result<(u64, bool), ZellijError> {
        if receipt
            .reported_id()
            .is_some_and(|tab_id| preexisting_tab_ids.contains(&tab_id))
        {
            return Err(ZellijError::InvalidOutput(format!(
                "{} was already present before the mutation; refusing to claim it",
                receipt.failure_reason()
            )));
        }

        let started = Instant::now();
        loop {
            let tab_probe_timeout = METADATA_TIMEOUT.min(timeout.saturating_sub(started.elapsed()));
            if let Some(tabs) = self.observe_creation_tabs(host, session, tab_probe_timeout)? {
                let candidates = tabs
                    .iter()
                    .filter(|tab| tab.name == name && !preexisting_tab_ids.contains(&tab.tab_id))
                    .collect::<Vec<_>>();
                if candidates.len() > 1 {
                    return Err(ZellijError::InvalidOutput(format!(
                        "new-tab produced {} new Zellij tabs named {name:?}; refusing to choose one",
                        candidates.len()
                    )));
                }
                if let Some(candidate) = candidates.first() {
                    if receipt
                        .reported_id()
                        .is_some_and(|reported| reported != candidate.tab_id)
                    {
                        return Err(ZellijError::InvalidOutput(format!(
                            "{} but the new tab named {name:?} has ID {}; refusing a stale or mismatched result",
                            receipt.failure_reason(),
                            candidate.tab_id
                        )));
                    }
                    let pane_probe_timeout =
                        METADATA_TIMEOUT.min(timeout.saturating_sub(started.elapsed()));
                    if let Some(panes) =
                        self.observe_creation_panes(host, session, pane_probe_timeout)?
                    {
                        let terminal_panes = panes
                            .into_iter()
                            .filter(|pane| pane.tab_id == candidate.tab_id && !pane.is_plugin)
                            .collect::<Vec<_>>();
                        if terminal_panes.len() > 1 {
                            return Err(ZellijError::InvalidOutput(format!(
                                "new tab {} has {} terminal panes; refusing to infer its launch pane",
                                candidate.tab_id,
                                terminal_panes.len()
                            )));
                        }
                        if let Some(pane) = terminal_panes.first() {
                            if pane.tab_name != name {
                                return Err(ZellijError::InvalidOutput(format!(
                                    "new tab {} is named {name:?}, but its terminal pane reports tab name {:?}",
                                    candidate.tab_id, pane.tab_name
                                )));
                            }
                            return Ok((
                                candidate.tab_id,
                                receipt.ownership_confirmed(candidate.tab_id),
                            ));
                        }
                    }
                } else if let Some(reported_id) = receipt.reported_id() {
                    if let Some(tab) = tabs.iter().find(|tab| tab.tab_id == reported_id) {
                        return Err(ZellijError::InvalidOutput(format!(
                            "new-tab reported tab ID {reported_id}, but that tab is named {:?} instead of {name:?}",
                            tab.name
                        )));
                    }
                }
            }

            if started.elapsed() >= timeout {
                return Err(ZellijError::InvalidOutput(format!(
                    "{}; no single new tab named {name:?} with one terminal pane became ready within {}ms; no retry was sent",
                    receipt.failure_reason(),
                    timeout.as_millis()
                )));
            }
            std::thread::sleep(
                TAB_CREATION_RECONCILE_POLL.min(timeout.saturating_sub(started.elapsed())),
            );
        }
    }

    fn observe_creation_tabs(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        probe_timeout: Duration,
    ) -> Result<Option<Vec<ZellijTab>>, ZellijError> {
        let output = match host.exec_timeout(&self.list_tabs_command(session)?, probe_timeout) {
            Ok(output) => output,
            Err(TransportError::CommandTimedOut { .. }) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if transient_metadata_result(&output, "Timeout listing tabs") {
            return Ok(None);
        }
        let output = checked(output, "list Zellij tabs")?;
        serde_json::from_slice(&output.stdout)
            .map(Some)
            .map_err(|error| ZellijError::InvalidOutput(format!("invalid tab JSON: {error}")))
    }

    fn observe_creation_panes(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        probe_timeout: Duration,
    ) -> Result<Option<Vec<ZellijPane>>, ZellijError> {
        let output = match host.exec_timeout(&self.list_panes_command(session)?, probe_timeout) {
            Ok(output) => output,
            Err(TransportError::CommandTimedOut { .. }) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if transient_metadata_result(&output, "Timeout listing panes") {
            return Ok(None);
        }
        let output = checked(output, "list Zellij panes")?;
        parse_panes(&output.stdout).map(Some)
    }
}
